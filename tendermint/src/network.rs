use crate::outside_deps::TendermintOutsideDeps;
use crate::tendermint::Tendermint;
use crate::utils::{
    aggregation_to_vote, AggregationResult, Checkpoint, ProposalResult, Step, VoteDecision,
    VoteResult,
};
use crate::TendermintError;
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_primitives::policy::TWO_THIRD_SLOTS;
use std::clone::Clone;

impl<
        ProposalTy: Clone + PartialEq + Hash + Unpin + 'static,
        ProofTy: Clone + Unpin + 'static,
        ResultTy: Unpin + 'static,
        DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
            + 'static,
    > Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy>
{
    pub(crate) async fn await_proposal(&mut self, round: u32) -> Result<(), TendermintError> {
        let proposal_res = self.deps.await_proposal(round).await?;

        match proposal_res {
            ProposalResult::Proposal(proposal, valid_round) => {
                if valid_round.is_none() {
                    self.state.current_proposal = Some(proposal);
                    self.state.current_checkpoint = Checkpoint::OnProposal;
                } else if valid_round.unwrap() < round
                    && self.has_2f1_prevotes(proposal.hash(), valid_round.unwrap())?
                {
                    self.state.current_proposal = Some(proposal);
                    self.state.current_proposal_vr = valid_round;
                    self.state.current_checkpoint = Checkpoint::OnPastProposal;
                } else {
                    // If we received an invalid proposal, and are not waiting for another, we might
                    // as well assume that we will timeout.
                    self.state.current_proposal = None;
                    self.state.current_checkpoint = Checkpoint::OnTimeoutPropose;
                }
            }
            ProposalResult::Timeout => {
                self.state.current_proposal = None;
                self.state.current_checkpoint = Checkpoint::OnTimeoutPropose;
            }
        }

        Ok(())
    }

    pub(crate) async fn broadcast_and_aggregate_prevote(
        &mut self,
        round: u32,
        decision: VoteDecision,
    ) -> Result<(), TendermintError> {
        let proposal = match decision {
            VoteDecision::Block => self.state.current_proposal.clone(),
            VoteDecision::Nil => None,
        };

        let proposal_hash = proposal.map(|p| p.hash());

        let prevote_agg = self
            .deps
            .broadcast_and_aggregate(round, Step::Prevote, proposal_hash.clone())
            .await?;

        let prevote = aggregation_to_vote(proposal_hash, prevote_agg);

        match prevote {
            VoteResult::Block(_) => {
                // Assuming that Handel only returns Block if there are 2f+1 prevotes for OUR
                // block, then here we are guaranteed that: 1) we have a proposal, 2) it is valid and
                // 3) the prevotes are for this proposal.
                self.state.current_checkpoint = Checkpoint::OnPolka;
            }
            VoteResult::Nil(_) => {
                self.state.current_checkpoint = Checkpoint::OnNilPolka;
            }
            VoteResult::Timeout => {
                self.state.current_checkpoint = Checkpoint::OnTimeoutPrevote;
            }
            VoteResult::NewRound(round) => {
                self.state.round = round;
                self.state.current_checkpoint = Checkpoint::StartRound;
            }
        }

        Ok(())
    }

    pub(crate) async fn broadcast_and_aggregate_precommit(
        &mut self,
        round: u32,
        decision: VoteDecision,
    ) -> Result<(), TendermintError> {
        let proposal = match decision {
            VoteDecision::Block => self.state.current_proposal.clone(),
            VoteDecision::Nil => None,
        };

        let proposal_hash = proposal.map(|p| p.hash());

        let precom_agg = self
            .deps
            .broadcast_and_aggregate(round, Step::Precommit, proposal_hash.clone())
            .await?;

        let precom = aggregation_to_vote(proposal_hash, precom_agg);

        match precom {
            VoteResult::Block(proof) => {
                // Again depends on how Handel treats votes for blocks different than the one
                // we voted for. But we only want to call on_2f1_block_precommits if the precommits
                // are for our block (so we can assemble it).
                self.state.current_proof = Some(proof);
                self.state.current_checkpoint = Checkpoint::OnDecision;
            }
            VoteResult::Nil(_) => {
                self.state.current_checkpoint = Checkpoint::OnTimeoutPrecommit;
            }
            VoteResult::Timeout => {
                self.state.current_checkpoint = Checkpoint::OnTimeoutPrecommit;
            }
            VoteResult::NewRound(round) => {
                self.state.round = round;
                self.state.current_checkpoint = Checkpoint::StartRound;
            }
        }

        Ok(())
    }

    // Check if you have the 2f+1 prevotes
    pub(crate) fn has_2f1_prevotes(
        &self,
        proposal_hash: Blake2sHash,
        round: u32,
    ) -> Result<bool, TendermintError> {
        let agg_result = self.deps.get_aggregation(round, Step::Prevote)?;

        let agg = match agg_result {
            AggregationResult::Aggregation(v) => v,
            AggregationResult::NewRound(_) => return Ok(false),
        };

        Ok(agg.get(&Some(proposal_hash)).map_or(0, |x| x.1) >= TWO_THIRD_SLOTS as usize)
    }
}
