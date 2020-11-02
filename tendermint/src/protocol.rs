use crate::tendermint_outside_deps::TendermintOutsideDeps;
use crate::tendermint_state::{TendermintState, TendermintStateProof};
use crate::{
    AggregationResult, ProposalResult, SingleDecision, Step, TendermintError, TendermintReturn,
};
use beserial::{Deserialize, Serialize};
use futures::channel::mpsc::{unbounded as mpsc_channel, UnboundedReceiver};
use macros::upgrade_weak;
use parking_lot::RwLock;
use std::clone::Clone;
use std::sync::{Arc, Weak};
use utils::mutable_once::MutableOnce;

pub struct Tendermint<
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
    ResultTy: Send + Sync + 'static,
    Deps: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>,
> {
    deps: Deps,
    weak_self: MutableOnce<Weak<Self>>,
    state: Arc<RwLock<TendermintState<ProposalTy, ProofTy, ResultTy>>>,
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
        });
        unsafe { this.weak_self.replace(Arc::downgrade(&this)) };
        this
    }

    pub fn expect_block(
        &self,
    ) -> UnboundedReceiver<TendermintReturn<ProposalTy, ProofTy, ResultTy>> {
        let (sender, receiver) = mpsc_channel();
        self.state.write().return_stream = Some(sender);

        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.start_round(0);
        });

        receiver
    }

    pub fn expect_block_from_state(
        &self,
        state_proof: TendermintStateProof<ProposalTy, ProofTy>,
    ) -> UnboundedReceiver<TendermintReturn<ProposalTy, ProofTy, ResultTy>> {
        let (sender, receiver) = mpsc_channel();
        self.state.write().return_stream = Some(sender);

        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);

            let mut invalid_state = false;

            match state_proof.step {
                Step::Propose => {
                    if state_proof.current_proof.is_some()
                        && this.deps.verify_proposal_state(
                            state_proof.round,
                            &state_proof.current_proof.unwrap(),
                        )
                    {
                        this.start_round(state_proof.round);
                    } else {
                        invalid_state = true;
                    }
                }
                Step::Prevote => {
                    if state_proof.current_proposal.is_some()
                        && this.deps.verify_prevote_state(
                            state_proof.round,
                            state_proof.current_proposal.unwrap(),
                            &state_proof.current_proof.unwrap(),
                        )
                    {
                        // Shouldn't this be able to vote nil also? Like on the Precommit step?
                        this.broadcast_and_aggregate_prevote(
                            state_proof.round,
                            SingleDecision::Block,
                        );
                    } else {
                        invalid_state = true;
                    }
                }
                Step::Precommit => {
                    if let Some(decision) = this.deps.verify_precommit_state(
                        state_proof.round,
                        state_proof.current_proposal,
                        &state_proof.current_proof.unwrap(),
                    ) {
                        this.broadcast_and_aggregate_precommit(state_proof.round, decision);
                    } else {
                        invalid_state = true;
                    }
                }
            };

            if invalid_state {
                if let Some(return_stream) = this.state.write().return_stream.as_mut() {
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
    fn start_round(&self, round: u32) {
        self.state.write().round = round;
        self.state.write().step = Step::Propose;

        if self.deps.is_our_turn(round) {
            let proposal =
                if self.state.read().valid_value.is_some() {
                    self.state.read().valid_value.clone().unwrap()
                } else {
                    Arc::new(self.deps.get_value(round).unwrap_or_else(|| {
                        panic!("Failed getting proposal value in round {}", round)
                    }))
                };

            self.broadcast_proposal(round, proposal.clone(), self.state.read().valid_round);

            // Update our proposal state
            self.state.write().current_proposal = Some(proposal);

            // Prevote for our own proposal
            self.state.write().step = Step::Prevote;
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Block);
        } else {
            self.await_proposal(round);
        }
    }

    // Lines  22-27
    fn on_proposal(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Propose);

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.is_none()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Block);
        } else {
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Nil);
        }

        self.state.write().step = Step::Prevote;

        // self.return_state_update();
    }

    // Lines 28-33.
    fn on_past_proposal(&self, round: u32, valid_round: Option<u32>) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Propose);

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.unwrap_or(0) <= valid_round.unwrap()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Block);
        } else {
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Nil);
        }

        self.state.write().step = Step::Prevote;

        // self.return_state_update();
    }

    // Lines 34-35: Handel takes care of waiting `timeoutPrevote` before returning
    // TODO: Maybe move broadcast_and_aggregate_prevote to here???

    // Lines 36-43
    fn on_polka(&self, round: u32) {
        // step is either prevote or precommit
        assert!(self.state.read().round == round && self.state.read().step != Step::Propose);

        if self.state.read().step == Step::Prevote {
            self.state.write().locked_value = self.state.read().current_proposal.clone();
            self.state.write().locked_round = Some(round);
            self.broadcast_and_aggregate_precommit(round, SingleDecision::Block);
            self.state.write().step = Step::Precommit;
        }

        self.state.write().valid_value = self.state.read().current_proposal.clone();
        self.state.write().valid_round = Some(round);
    }

    // Lines 44-46
    fn on_nil_polka(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Prevote);

        self.broadcast_and_aggregate_precommit(round, SingleDecision::Nil);
        self.state.write().step = Step::Precommit;
    }

    // Lines 47-48 Handel takes care of waiting `timeoutPrecommit` before returning
    // TODO: Maybe move broadcast_and_aggregate_precommit to here???

    // Lines 49-54
    // We only handle precommits for our current round and current proposal in this function. The
    // validator crate is always listening for completed blocks. The purpose of this function is just
    // to assemble the block if we can.
    fn on_2f1_block_precommits(&self, round: u32, proof: ProofTy) {
        assert_eq!(self.state.read().round, round);

        let block = self
            .deps
            .assemble_block(self.state.read().current_proposal.clone().unwrap(), proof);

        if let Some(return_stream) = self.state.write().return_stream.as_mut() {
            if return_stream
                .unbounded_send(TendermintReturn::Result(block))
                .is_err()
            {
                warn!("Failed sending/returning completed macro block")
            }
            return_stream.close_channel();
        }
    }

    // Lines 55-56 We instead trigger a validator state resync if we get too many unexpected level updates.
    // TODO: This supposedly is handled by Handel. But how?

    // Lines 57-60
    fn on_timeout_propose(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Propose);

        self.broadcast_and_aggregate_prevote(round, SingleDecision::Nil);

        self.state.write().step = Step::Prevote;
    }

    // Lines 61-64
    fn on_timeout_prevote(&self, round: u32) {
        assert!(self.state.read().round == round && self.state.read().step == Step::Prevote);

        self.broadcast_and_aggregate_precommit(round, SingleDecision::Nil);

        self.state.write().step = Step::Precommit;
    }

    // Lines 65-67
    fn on_timeout_precommit(&self, round: u32) {
        assert_eq!(self.state.read().round, round);

        self.start_round(round + 1);
    }

    fn broadcast_proposal(&self, round: u32, proposal: Arc<ProposalTy>, valid_round: Option<u32>) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.deps
                .broadcast_proposal(round, proposal, valid_round)
                .await;
        });
    }

    fn await_proposal(&self, round: u32) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.await_proposal_async(round).await;
        });
    }

    async fn await_proposal_async(&self, round: u32) {
        let proposal_res = self.deps.await_proposal(round).await;

        match proposal_res {
            ProposalResult::Proposal(proposal, valid_round) => {
                if valid_round.is_none() {
                    self.state.write().current_proposal = Some(Arc::new(proposal));
                    self.on_proposal(round);
                } else if valid_round.unwrap() < round {
                    // TODO: You also need to check if you have the 2f+1 prevotes
                    self.state.write().current_proposal = Some(Arc::new(proposal));
                    self.on_past_proposal(round, valid_round);
                } else {
                    // If we received an invalid proposal, and are not waiting for another, we might
                    // as well assume that we will timeout.
                    self.state.write().current_proposal = None;
                    self.on_timeout_propose(round);
                }
            }
            ProposalResult::Timeout => {
                self.state.write().current_proposal = None;
                self.on_timeout_propose(round);
            }
        }
    }

    fn broadcast_and_aggregate_prevote(&self, round: u32, our_decision: SingleDecision) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.broadcast_and_aggregate_prevote_async(round, our_decision)
                .await;
        });
    }

    async fn broadcast_and_aggregate_prevote_async(&self, round: u32, decision: SingleDecision) {
        let proposal = self.state.read().current_proposal.clone();
        let prevote_res = self
            .deps
            .broadcast_and_aggregate(round, proposal, Step::Prevote, decision)
            .await;

        match prevote_res {
            AggregationResult::Block(proof) => {
                // TODO: Assuming that Handel only returns Block if there are 2f+1 prevotes for OUR
                // block, then here we are guaranteed that: 1) we have a proposal, 2) it is valid and
                // 3) the prevotes are for this proposal.
                // What about when we prevoted Nil???
                self.state.write().current_proof = Some(proof);
                self.on_polka(round);
            }
            AggregationResult::Nil(proof) => {
                self.state.write().current_proof = Some(proof);
                self.on_nil_polka(round);
            }
            AggregationResult::Timeout(proof) => {
                self.state.write().current_proof = Some(proof);
                self.on_timeout_prevote(round);
            }
        }
    }

    fn broadcast_and_aggregate_precommit(&self, round: u32, our_decision: SingleDecision) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.broadcast_and_aggregate_precommit_async(round, our_decision)
                .await;
        });
    }

    async fn broadcast_and_aggregate_precommit_async(&self, round: u32, decision: SingleDecision) {
        let proposal = self.state.read().current_proposal.clone();
        let precom_res = self
            .deps
            .broadcast_and_aggregate(round, proposal, Step::Precommit, decision)
            .await;

        match precom_res {
            AggregationResult::Block(proof) => {
                // TODO: Again depends on how Handel treats votes for blocks different than the one
                // we voted for. But we only want to call on_2f1_block_precommits if the precommits
                // are for our block (so we can assemble it).
                self.on_2f1_block_precommits(round, proof);
            }
            AggregationResult::Nil(_) => {
                self.on_timeout_precommit(round);
            }
            AggregationResult::Timeout(_) => {
                self.on_timeout_precommit(round);
            }
        }
    }

    fn return_state_update(&self) {
        let state = self.state.read();

        let state_update = TendermintReturn::StateUpdate(TendermintStateProof {
            round: state.round,
            step: state.step,
            locked_value: state.locked_value.clone(),
            locked_round: state.locked_round,
            valid_value: state.valid_value.clone(),
            valid_round: state.valid_round,
            current_proposal: state.current_proposal.clone(),
            current_proof: state.current_proof.clone(),
        });

        drop(state);

        if let Some(return_stream) = self.state.write().return_stream.as_mut() {
            if return_stream.unbounded_send(state_update).is_err() {
                warn!("Failed sending/returning tendermint state update")
            }
        }
    }
}
