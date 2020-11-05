use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::utils::{Step, TendermintError, TendermintReturn, VoteDecision};
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
            this.start_round(0);
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

            let mut invalid_state = false;

            match state.step {
                Step::Propose => {
                    if state.current_proof.is_some()
                        && this
                            .deps
                            .verify_proposal_state(state.round, &state.current_proof.unwrap())
                    {
                        this.start_round(state.round);
                    } else {
                        invalid_state = true;
                    }
                }
                Step::Prevote => {
                    if state.current_proposal.is_some()
                        && this.deps.verify_prevote_state(
                            state.round,
                            state.current_proposal.unwrap(),
                            &state.current_proof.unwrap(),
                        )
                    {
                        // Shouldn't this be able to vote nil also? Like on the Precommit step?
                        this.broadcast_and_aggregate_prevote(state.round, VoteDecision::Block);
                    } else {
                        invalid_state = true;
                    }
                }
                Step::Precommit => {
                    if let Some(decision) = this.deps.verify_precommit_state(
                        state.round,
                        state.current_proposal,
                        &state.current_proof.unwrap(),
                    ) {
                        this.broadcast_and_aggregate_precommit(state.round, decision);
                    } else {
                        invalid_state = true;
                    }
                }
            };

            if invalid_state {
                if let Some(return_stream) = &this.return_stream {
                    if let Err(_) = return_stream
                        .unbounded_send(TendermintReturn::Error(TendermintError::BadInitState))
                    {
                        warn!("Failed initializing Tendermint. Invalid state.")
                    }
                    return_stream.close_channel();
                }
            }
        });

        receiver
    }

    // Lines 11-21
    pub(crate) fn start_round(&self, round: u32) {
        self.state.write().round = round;
        self.state.write().step = Step::Propose;
        self.state.write().current_proposal = None;
        self.state.write().current_proof = None;

        if self.deps.is_our_turn(round) {
            let proposal = if self.state.read().valid_value.is_some() {
                self.state.read().valid_value.clone().unwrap()
            } else {
                Arc::new(self.deps.get_value(round))
            };

            self.broadcast_proposal(round, proposal.clone(), self.state.read().valid_round);

            // Update our proposal state
            self.state.write().current_proposal = Some(proposal);

            // Prevote for our own proposal
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block);
            self.state.write().step = Step::Prevote;
            self.return_state_update();
        } else {
            self.await_proposal(round);
        }
    }

    // Lines  22-27
    pub(crate) fn on_proposal(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Propose);

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.is_none()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block);
        } else {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil);
        }

        self.state.write().step = Step::Prevote;

        self.return_state_update();
    }

    // Lines 28-33.
    pub(crate) fn on_past_proposal(&self, round: u32, valid_round: Option<u32>) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Propose);

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.unwrap_or(0) <= valid_round.unwrap()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block);
        } else {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil);
        }

        self.state.write().step = Step::Prevote;

        self.return_state_update();
    }

    // Lines 34-35: Handel takes care of waiting `timeoutPrevote` before returning

    // Lines 36-43
    pub(crate) fn on_polka(&self, round: u32) {
        // step is either prevote or precommit
        assert!(self.state.read().round == round && self.state.read().step != Step::Propose);

        if self.state.read().step == Step::Prevote {
            self.state.write().locked_value = self.state.read().current_proposal.clone();
            self.state.write().locked_round = Some(round);
            self.broadcast_and_aggregate_precommit(round, VoteDecision::Block);
            self.state.write().step = Step::Precommit;
        }

        self.state.write().valid_value = self.state.read().current_proposal.clone();
        self.state.write().valid_round = Some(round);
        self.return_state_update();
    }

    // Lines 44-46
    pub(crate) fn on_nil_polka(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Prevote);

        self.broadcast_and_aggregate_precommit(round, VoteDecision::Nil);
        self.state.write().step = Step::Precommit;
        self.return_state_update();
    }

    // Lines 47-48 Handel takes care of waiting `timeoutPrecommit` before returning

    // Lines 49-54
    // We only handle precommits for our current round and current proposal in this function. The
    // validator crate is always listening for completed blocks. The purpose of this function is just
    // to assemble the block if we can.
    pub(crate) fn on_2f1_block_precommits(&self, round: u32, proof: ProofTy) {
        assert_eq!(self.state.read().round, round);

        let block = self
            .deps
            .assemble_block(self.state.read().current_proposal.clone().unwrap(), proof);

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
    pub(crate) fn on_timeout_propose(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Propose);

        self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil);

        self.state.write().step = Step::Prevote;

        self.return_state_update();
    }

    // Lines 61-64
    pub(crate) fn on_timeout_prevote(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Prevote);

        self.broadcast_and_aggregate_precommit(round, VoteDecision::Nil);

        self.state.write().step = Step::Precommit;

        self.return_state_update();
    }

    // Lines 65-67
    pub(crate) fn on_timeout_precommit(&self, round: u32) {
        assert_eq!(self.state.read().round, round);

        self.start_round(round + 1);
    }
}
