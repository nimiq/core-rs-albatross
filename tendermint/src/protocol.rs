use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::utils::{Checkpoint, Step, TendermintError, TendermintReturn, VoteDecision};
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
            this.start_round();
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

            match state.current_checkpoint {
                Checkpoint::StartRound => this.start_round(),
                Checkpoint::OnProposal => this.on_proposal(),
                Checkpoint::OnPastProposal => this.on_past_proposal(),
                Checkpoint::OnPolka => this.on_polka(),
                Checkpoint::OnNilPolka => this.on_nil_polka(),
                Checkpoint::OnDecision => this.on_decision(),
                Checkpoint::OnTimeoutPropose => this.on_timeout_propose(),
                Checkpoint::OnTimeoutPrevote => this.on_timeout_prevote(),
                Checkpoint::OnTimeoutPrecommit => this.on_timeout_precommit(),
            }
        });

        receiver
    }

    // Lines 11-21
    pub(crate) fn start_round(&self) {
        self.state.write().current_checkpoint = Checkpoint::StartRound;
        self.return_state_update();

        self.state.write().current_proposal = None;
        self.state.write().current_proposal_vr = None;
        self.state.write().current_proof = None;

        self.state.write().step = Step::Propose;

        if self.deps.is_our_turn(self.state.read().round) {
            let proposal = if self.state.read().valid_value.is_some() {
                self.state.read().valid_value.clone().unwrap()
            } else {
                Arc::new(self.deps.get_value(self.state.read().round))
            };

            // Update and send our proposal state
            self.state.write().current_proposal = Some(proposal.clone());
            self.state.write().current_proposal_vr = self.state.read().valid_round;
            self.broadcast_proposal(
                self.state.read().round,
                proposal,
                self.state.read().valid_round,
            );

            // Prevote for our own proposal
            self.state.write().step = Step::Prevote;
            self.broadcast_and_aggregate_prevote(self.state.read().round, VoteDecision::Block);
        } else {
            self.await_proposal(self.state.read().round);
        }
    }

    // Lines  22-27
    pub(crate) fn on_proposal(&self) {
        assert_eq!(self.state.read().step, Step::Propose);

        self.state.write().current_checkpoint = Checkpoint::OnProposal;
        self.return_state_update();

        self.state.write().step = Step::Prevote;

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.is_none()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(self.state.read().round, VoteDecision::Block);
        } else {
            self.broadcast_and_aggregate_prevote(self.state.read().round, VoteDecision::Nil);
        }
    }

    // Lines 28-33.
    pub(crate) fn on_past_proposal(&self) {
        assert_eq!(self.state.read().step, Step::Propose);

        self.state.write().current_checkpoint = Checkpoint::OnPastProposal;
        self.return_state_update();

        self.state.write().step = Step::Prevote;

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.unwrap_or(0)
                <= self.state.read().current_proposal_vr.unwrap()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(self.state.read().round, VoteDecision::Block);
        } else {
            self.broadcast_and_aggregate_prevote(self.state.read().round, VoteDecision::Nil);
        }
    }

    // Lines 34-35: Handel takes care of waiting `timeoutPrevote` before returning

    // Lines 36-43
    pub(crate) fn on_polka(&self) {
        // step is either prevote or precommit
        assert_ne!(self.state.read().step, Step::Propose);

        self.state.write().current_checkpoint = Checkpoint::OnPolka;
        self.return_state_update();

        self.state.write().valid_value = self.state.read().current_proposal.clone();
        self.state.write().valid_round = Some(self.state.read().round);

        if self.state.read().step == Step::Prevote {
            self.state.write().locked_value = self.state.read().current_proposal.clone();
            self.state.write().locked_round = Some(self.state.read().round);
            self.state.write().step = Step::Precommit;
            self.broadcast_and_aggregate_precommit(self.state.read().round, VoteDecision::Block);
        }
    }

    // Lines 44-46
    pub(crate) fn on_nil_polka(&self) {
        assert_eq!(self.state.read().step, Step::Prevote);

        self.state.write().current_checkpoint = Checkpoint::OnNilPolka;
        self.return_state_update();

        self.state.write().step = Step::Precommit;

        self.broadcast_and_aggregate_precommit(self.state.read().round, VoteDecision::Nil);
    }

    // Lines 47-48 Handel takes care of waiting `timeoutPrecommit` before returning

    // Lines 49-54
    // We only handle precommits for our current round and current proposal in this function. The
    // validator crate is always listening for completed blocks. The purpose of this function is just
    // to assemble the block if we can.
    pub(crate) fn on_decision(&self) {
        self.state.write().current_checkpoint = Checkpoint::OnDecision;
        self.return_state_update();

        let block = self.deps.assemble_block(
            self.state.read().current_proposal.clone().unwrap(),
            self.state.read().current_proof.clone().unwrap(),
        );

        if let Some(return_stream) = &self.return_stream {
            if return_stream
                .unbounded_send(TendermintReturn::Result(block))
                .is_err()
            {
                warn!("Failed sending/returning completed macro block")
            }
            return_stream.close_channel();
        }
    }

    // Lines 55-56 Go to next round if f+1
    // Not here but anytime you handle VoteResult.

    // Lines 57-60
    pub(crate) fn on_timeout_propose(&self) {
        assert_eq!(self.state.read().step, Step::Propose);

        self.state.write().current_checkpoint = Checkpoint::OnTimeoutPropose;
        self.return_state_update();

        self.state.write().step = Step::Prevote;

        self.broadcast_and_aggregate_prevote(self.state.read().round, VoteDecision::Nil);
    }

    // Lines 61-64
    pub(crate) fn on_timeout_prevote(&self) {
        assert_eq!(self.state.read().step, Step::Prevote);

        self.state.write().current_checkpoint = Checkpoint::OnTimeoutPrevote;
        self.return_state_update();

        self.state.write().step = Step::Precommit;

        self.broadcast_and_aggregate_precommit(self.state.read().round, VoteDecision::Nil);
    }

    // Lines 65-67
    pub(crate) fn on_timeout_precommit(&self) {
        self.state.write().current_checkpoint = Checkpoint::OnTimeoutPrevote;
        self.return_state_update();

        self.state.write().round += 1;

        self.start_round();
    }
}
