use crate::outside_deps::TendermintOutsideDeps;
use crate::tendermint::Tendermint;
use crate::utils::{Checkpoint, Step, TendermintReturn, VoteDecision};
use beserial::{Deserialize, Serialize};
use nimiq_hash::Hash;
use std::clone::Clone;
use std::sync::Arc;

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
    // Lines 11-21
    pub(crate) async fn start_round(&self) {
        self.state.write().current_proposal = None;
        self.state.write().current_proposal_vr = None;
        self.state.write().current_proof = None;

        self.state.write().step = Step::Propose;

        let round = self.state.read().round;
        let valid_round = self.state.read().valid_round;

        if self.deps.is_our_turn(round) {
            let proposal = if self.state.read().valid_value.is_some() {
                self.state.read().valid_value.clone().unwrap()
            } else {
                Arc::new(self.deps.get_value(round))
            };

            // Update and send our proposal state
            self.state.write().current_proposal = Some(proposal.clone());
            self.state.write().current_proposal_vr = valid_round;
            self.deps
                .broadcast_proposal(round, proposal, valid_round)
                .await;

            // Prevote for our own proposal
            self.state.write().step = Step::Prevote;
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block)
                .await;
        } else {
            self.await_proposal(round).await;
        }
    }

    // Lines  22-27
    pub(crate) async fn on_proposal(&self) {
        assert_eq!(self.state.read().step, Step::Propose);

        self.state.write().step = Step::Prevote;

        let round = self.state.read().round;

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.is_none()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block)
                .await;
        } else {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil)
                .await;
        }
    }

    // Lines 28-33.
    pub(crate) async fn on_past_proposal(&self) {
        assert_eq!(self.state.read().step, Step::Propose);

        self.state.write().step = Step::Prevote;

        let round = self.state.read().round;

        if self
            .deps
            .is_valid(self.state.read().current_proposal.clone().unwrap())
            && (self.state.read().locked_round.unwrap_or(0)
                <= self.state.read().current_proposal_vr.unwrap()
                || self.state.read().locked_value == self.state.read().current_proposal)
        {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block)
                .await;
        } else {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil)
                .await;
        }
    }

    // Lines 34-35: Handel takes care of waiting `timeoutPrevote` before returning

    // Lines 36-43
    pub(crate) async fn on_polka(&self) {
        // step is either prevote or precommit
        assert_ne!(self.state.read().step, Step::Propose);

        let round = self.state.read().round;

        self.state.write().valid_value = self.state.read().current_proposal.clone();
        self.state.write().valid_round = Some(round);

        // TODO: What happens if the step is not prevote? Can this even happen?
        if self.state.read().step == Step::Prevote {
            self.state.write().locked_value = self.state.read().current_proposal.clone();
            self.state.write().locked_round = Some(round);
            self.state.write().step = Step::Precommit;
            self.broadcast_and_aggregate_precommit(round, VoteDecision::Block)
                .await;
        }
    }

    // Lines 44-46
    pub(crate) async fn on_nil_polka(&self) {
        assert_eq!(self.state.read().step, Step::Prevote);

        self.state.write().step = Step::Precommit;

        let round = self.state.read().round;

        self.broadcast_and_aggregate_precommit(round, VoteDecision::Nil)
            .await;
    }

    // Lines 47-48 Handel takes care of waiting `timeoutPrecommit` before returning

    // Lines 49-54
    // We only handle precommits for our current round and current proposal in this function. The
    // validator crate is always listening for completed blocks. The purpose of this function is just
    // to assemble the block if we can.
    pub(crate) fn on_decision(&self) {
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

        self.state.write().current_checkpoint = Checkpoint::Finished;
    }

    // Lines 55-56 Go to next round if f+1
    // Not here but anytime you handle VoteResult.

    // Lines 57-60
    pub(crate) async fn on_timeout_propose(&self) {
        assert_eq!(self.state.read().step, Step::Propose);

        self.state.write().step = Step::Prevote;

        let round = self.state.read().round;

        self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil)
            .await;
    }

    // Lines 61-64
    pub(crate) async fn on_timeout_prevote(&self) {
        assert_eq!(self.state.read().step, Step::Prevote);

        self.state.write().step = Step::Precommit;

        let round = self.state.read().round;

        self.broadcast_and_aggregate_precommit(round, VoteDecision::Nil)
            .await;
    }

    // Lines 65-67
    pub(crate) fn on_timeout_precommit(&self) {
        self.state.write().round += 1;

        self.state.write().current_checkpoint = Checkpoint::StartRound;
    }
}
