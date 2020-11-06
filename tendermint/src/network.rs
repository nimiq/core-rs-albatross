use crate::outside_deps::TendermintOutsideDeps;
use crate::protocol::Tendermint;
use crate::state::TendermintState;
use crate::utils::{
    aggregation_to_vote, AggregationResult, ProposalResult, Step, TendermintError,
    TendermintReturn, VoteDecision, VoteResult,
};
use beserial::{Deserialize, Serialize};
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_macros::upgrade_weak;
use nimiq_primitives::policy::TWO_THIRD_SLOTS;
use std::clone::Clone;
use std::sync::Arc;

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
    pub(crate) fn broadcast_proposal(
        &self,
        round: u32,
        proposal: Arc<ProposalTy>,
        valid_round: Option<u32>,
    ) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.deps
                .broadcast_proposal(round, proposal, valid_round)
                .await;
        });
    }

    pub(crate) fn await_proposal(&self, round: u32) {
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
                    self.on_proposal();
                } else if valid_round.unwrap() < round
                    && self.has_2f1_prevotes(proposal.hash(), valid_round.unwrap())
                {
                    self.state.write().current_proposal = Some(Arc::new(proposal));
                    self.state.write().current_proposal_vr = valid_round;
                    self.on_past_proposal();
                } else {
                    // If we received an invalid proposal, and are not waiting for another, we might
                    // as well assume that we will timeout.
                    self.state.write().current_proposal = None;
                    self.on_timeout_propose();
                }
            }
            ProposalResult::Timeout => {
                self.state.write().current_proposal = None;
                self.on_timeout_propose();
            }
        }
    }

    pub(crate) fn broadcast_and_aggregate_prevote(&self, round: u32, decision: VoteDecision) {
        let weak_self = self.weak_self.clone();

        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.broadcast_and_aggregate_prevote_async(round, decision)
                .await;
        });
    }

    async fn broadcast_and_aggregate_prevote_async(&self, round: u32, decision: VoteDecision) {
        let proposal = match decision {
            VoteDecision::Block => self.state.read().current_proposal.clone(),
            VoteDecision::Nil => None,
        };

        let proposal_hash = proposal.map(|p| p.hash());

        let prevote_agg = self
            .deps
            .broadcast_and_aggregate(round, Step::Prevote, proposal_hash.clone())
            .await
            .unwrap();

        let prevote = aggregation_to_vote(proposal_hash, prevote_agg);

        match prevote {
            VoteResult::Block(_) => {
                // Assuming that Handel only returns Block if there are 2f+1 prevotes for OUR
                // block, then here we are guaranteed that: 1) we have a proposal, 2) it is valid and
                // 3) the prevotes are for this proposal.
                self.on_polka();
            }
            VoteResult::Nil(_) => {
                self.on_nil_polka();
            }
            VoteResult::Timeout => {
                self.on_timeout_prevote();
            }
            VoteResult::NewRound(round) => {
                self.state.write().round = round;
                self.start_round();
            }
        }
    }

    pub(crate) fn broadcast_and_aggregate_precommit(&self, round: u32, decision: VoteDecision) {
        let weak_self = self.weak_self.clone();

        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.broadcast_and_aggregate_precommit_async(round, decision)
                .await;
        });
    }

    async fn broadcast_and_aggregate_precommit_async(&self, round: u32, decision: VoteDecision) {
        let proposal = match decision {
            VoteDecision::Block => self.state.read().current_proposal.clone(),
            VoteDecision::Nil => None,
        };

        let proposal_hash = proposal.map(|p| p.hash());

        let precom_agg = self
            .deps
            .broadcast_and_aggregate(round, Step::Precommit, proposal_hash.clone())
            .await
            .unwrap();

        let precom = aggregation_to_vote(proposal_hash, precom_agg);

        match precom {
            VoteResult::Block(proof) => {
                // Again depends on how Handel treats votes for blocks different than the one
                // we voted for. But we only want to call on_2f1_block_precommits if the precommits
                // are for our block (so we can assemble it).
                self.state.write().current_proof = Some(proof);
                self.on_decision();
            }
            VoteResult::Nil(_) => {
                self.on_timeout_precommit();
            }
            VoteResult::Timeout => {
                self.on_timeout_precommit();
            }
            VoteResult::NewRound(round) => {
                self.state.write().round = round;
                self.start_round();
            }
        }
    }

    pub(crate) fn return_state_update(&self) {
        let state = self.state.read();

        let state_update = TendermintReturn::StateUpdate(TendermintState {
            round: state.round,
            step: state.step,
            locked_value: state.locked_value.clone(),
            locked_round: state.locked_round,
            valid_value: state.valid_value.clone(),
            valid_round: state.valid_round,
            current_checkpoint: state.current_checkpoint,
            current_proposal: state.current_proposal.clone(),
            current_proposal_vr: state.current_proposal_vr,
            current_proof: state.current_proof.clone(),
        });

        drop(state);

        if let Some(return_stream) = &self.return_stream {
            if return_stream.unbounded_send(state_update).is_err() {
                warn!("Failed sending/returning tendermint state update")
            }
        }
    }

    // Check if you have the 2f+1 prevotes
    pub(crate) fn has_2f1_prevotes(&self, proposal_hash: Blake2sHash, round: u32) -> bool {
        let agg_result = match self.deps.get_aggregation(round, Step::Prevote) {
            Some(v) => v,
            None => return false,
        };

        let agg = match agg_result {
            AggregationResult::Aggregation(v) => v,
            AggregationResult::NewRound(_) => return false,
        };

        agg.get(&Some(proposal_hash)).map_or(0, |x| x.1) >= TWO_THIRD_SLOTS as usize
    }
}
