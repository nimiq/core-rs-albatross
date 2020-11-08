use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::utils::{Checkpoint, TendermintError, TendermintReturn};
use beserial::{Deserialize, Serialize};
use futures::channel::mpsc::{unbounded as mpsc_channel, UnboundedReceiver, UnboundedSender};
use nimiq_hash::Hash;
use nimiq_macros::upgrade_weak;
use nimiq_utils::mutable_once::MutableOnce;
use parking_lot::RwLock;
use std::clone::Clone;
use std::sync::{Arc, Weak};

pub struct Tendermint<
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
    ResultTy: Send + Sync + 'static,
    Deps: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>,
> {
    pub(crate) deps: Deps,
    pub(crate) weak_self: MutableOnce<Weak<Self>>,
    pub(crate) state: Arc<RwLock<TendermintState<ProposalTy, ProofTy>>>,
    pub(crate) return_stream:
        Option<UnboundedSender<TendermintReturn<ProposalTy, ProofTy, ResultTy>>>,
}

// This Tendermint implementation tries to stick to the pseudocode of https://arxiv.org/abs/1807.04938v3
impl<
        ProposalTy: Clone
            + Eq
            + PartialEq
            + Serialize
            + Deserialize
            + Send
            + Sync
            + Hash
            + 'static
            + std::fmt::Display,
        ProofTy: Clone + Send + Sync + 'static,
        ResultTy: Send + Sync + 'static,
        DepsTy: Send
            + Sync
            + TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
            + 'static,
    > Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy>
{
    pub fn new(deps: DepsTy) -> Arc<Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy>> {
        let this = Arc::new(Self {
            deps,
            weak_self: MutableOnce::new(Weak::new()),
            state: Arc::new(RwLock::new(TendermintState::new())),
            return_stream: None,
        });
        unsafe { this.weak_self.replace(Arc::downgrade(&this)) };
        this
    }

    pub fn expect_block(
        &mut self,
    ) -> UnboundedReceiver<TendermintReturn<ProposalTy, ProofTy, ResultTy>> {
        let (sender, receiver) = mpsc_channel();
        self.return_stream = Some(sender);

        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.main_loop().await;
        });

        receiver
    }

    pub fn expect_block_from_state(
        &mut self,
        state: TendermintState<ProposalTy, ProofTy>,
    ) -> UnboundedReceiver<TendermintReturn<ProposalTy, ProofTy, ResultTy>> {
        let (sender, receiver) = mpsc_channel();
        self.return_stream = Some(sender);

        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);

            if !this.deps.verify_state(state.clone()) {
                if let Some(return_stream) = &this.return_stream {
                    if let Err(_) = return_stream
                        .unbounded_send(TendermintReturn::Error(TendermintError::BadInitState))
                    {
                        warn!("Failed initializing Tendermint. Invalid state.")
                    }
                    return_stream.close_channel();
                }
            }

            this.state.write().round = state.round;
            this.state.write().step = state.step;
            this.state.write().locked_value = state.locked_value;
            this.state.write().locked_round = state.locked_round;
            this.state.write().valid_value = state.valid_value;
            this.state.write().valid_round = state.valid_round;
            this.state.write().current_checkpoint = state.current_checkpoint;
            this.state.write().current_proposal = state.current_proposal;
            this.state.write().current_proposal_vr = state.current_proposal_vr;
            this.state.write().current_proof = state.current_proof;

            this.main_loop().await;
        });

        receiver
    }

    pub(crate) async fn main_loop(&self) {
        loop {
            let checkpoint = self.state.read().current_checkpoint;

            match checkpoint {
                Checkpoint::StartRound => self.start_round().await,
                Checkpoint::OnProposal => self.on_proposal().await,
                Checkpoint::OnPastProposal => self.on_past_proposal().await,
                Checkpoint::OnPolka => self.on_polka().await,
                Checkpoint::OnNilPolka => self.on_nil_polka().await,
                Checkpoint::OnDecision => self.on_decision(),
                Checkpoint::OnTimeoutPropose => self.on_timeout_propose().await,
                Checkpoint::OnTimeoutPrevote => self.on_timeout_prevote().await,
                Checkpoint::OnTimeoutPrecommit => self.on_timeout_precommit(),
                Checkpoint::Finished => return,
            }

            self.return_state_update();
        }
    }
}
