use crate::outside_deps::TendermintOutsideDeps;
use crate::tendermint::Tendermint;
use crate::utils::{
    aggregation_to_vote, has_2f1_votes, Checkpoint, ProposalResult, Step, VoteDecision, VoteResult,
};
use crate::TendermintError;
use crate::{ProofTrait, ProposalTrait, ResultTrait};

/// This section implements methods to interface with TendermintOutsideDeps. All of them get or
/// broadcast some information from/to the network, processes it and then updates Tendermint's state.
impl<
        ProposalTy: ProposalTrait,
        ProofTy: ProofTrait,
        ResultTy: ResultTrait,
        DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
            + 'static,
    > Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy>
{
    /// Wait for a proposal for a given round from that round's proposer. The proposal that we
    /// receive might or not be valid, validity of the proposal is checked in the protocol methods.
    pub(crate) async fn await_proposal(&mut self, round: u32) -> Result<(), TendermintError> {
        // Wait for the proposal and receive ProposalResult.
        let proposal_res = self.deps.await_proposal(round).await?;

        match proposal_res {
            // If you received a proposal...
            ProposalResult::Proposal(proposal, valid_round) => {
                // ...and the valid round sent in the proposal is None (equal to -1 in the protocol),
                // save the proposal and set the state to OnProposal.
                if valid_round.is_none() {
                    self.state.current_proposal = Some(proposal);
                    self.state.current_checkpoint = Checkpoint::OnProposal;
                // ...and the valid round sent in the proposal is equal or greater than zero but
                // smaller than the current round, and we have 2f+1 prevotes for this proposal from
                // that valid round, save the proposal (including valid round) and set
                // the state to OnPastProposal.
                } else if valid_round.unwrap() < round
                    && has_2f1_votes(
                        proposal.hash(),
                        self.deps
                            .get_aggregation(valid_round.unwrap(), Step::Prevote)?,
                    )
                {
                    self.state.current_proposal = Some(proposal);
                    self.state.current_proposal_vr = valid_round;
                    self.state.current_checkpoint = Checkpoint::OnPastProposal;
                // ...and the valid round is invalid, we are not waiting for another proposal from
                // from this proposer, so we just assume that we will timeout.
                } else {
                    self.state.current_checkpoint = Checkpoint::OnTimeoutPropose;
                }
            }
            // If we timed out, we set Tendermint's state to OnTimeoutPropose.
            ProposalResult::Timeout => {
                self.state.current_checkpoint = Checkpoint::OnTimeoutPropose;
            }
        }

        Ok(())
    }

    /// Broadcast our prevote for a given proposal for a given round and wait for 2f+1 prevotes from
    /// other nodes.
    pub(crate) async fn broadcast_and_aggregate_prevote(
        &mut self,
        round: u32,
        decision: VoteDecision,
    ) -> Result<(), TendermintError> {
        // The TendermintOutsideDeps function to broadcast and aggregate votes only accepts the
        // proposal hash as input (it has no concept of voting Block or Nil, it just aggregates
        // signed messages from other nodes), so first we transform our voting decision into a
        // proposal option...
        let proposal = match decision {
            VoteDecision::Block => self.state.current_proposal.clone(),
            VoteDecision::Nil => None,
        };

        // ...then we hash it...
        let proposal_hash = proposal.map(|p| p.hash());

        // ... and we broadcast it. The function will wait for 2f+1 prevotes, aggregate them and
        // return.
        let prevote_agg = self
            .deps
            .broadcast_and_aggregate(round, Step::Prevote, proposal_hash.clone())
            .await?;

        // We transform the aggregation we got into an actual vote result. See the function for more
        // details.
        let prevote = aggregation_to_vote(proposal_hash, prevote_agg);

        // Match the vote result and update Tendermint's state.
        match prevote {
            VoteResult::Block(_) => {
                // We only get the Block result if there were 2f+1 prevotes for the proposal we
                // voted on (not Nil or another proposal), so we are guaranteed that:
                //      1) we have a proposal,
                //      2) it is valid, and
                //      3) the 2f+1 prevotes are for this proposal.
                self.state.current_checkpoint = Checkpoint::OnPolka;
            }
            VoteResult::Nil(_) => {
                // We get the Nil result if there were 2f+1 prevotes for Nil.
                self.state.current_checkpoint = Checkpoint::OnNilPolka;
            }
            VoteResult::Timeout => {
                // In all other cases, we get the Timeout result.
                self.state.current_checkpoint = Checkpoint::OnTimeoutPrevote;
            }
            VoteResult::NewRound(round) => {
                // If the network got f+1 messages for a round greater than our current one, we are
                // warned with the NewRound result and we start a new round.
                // This corresponds to lines 55-56 of Tendermint consensus algorithm.
                self.state.round = round;
                self.state.current_checkpoint = Checkpoint::StartRound;
            }
        }

        Ok(())
    }

    /// Broadcast our prevote for a given proposal for a given round and wait for 2f+1 prevotes from
    /// other nodes.
    pub(crate) async fn broadcast_and_aggregate_precommit(
        &mut self,
        round: u32,
        decision: VoteDecision,
    ) -> Result<(), TendermintError> {
        // The TendermintOutsideDeps function to broadcast and aggregate votes only accepts the
        // proposal hash as input (it has no concept of voting Block or Nil, it just aggregates
        // signed messages from other nodes), so first we transform our voting decision into a
        // proposal option...
        let proposal = match decision {
            VoteDecision::Block => self.state.current_proposal.clone(),
            VoteDecision::Nil => None,
        };

        // ...then we hash it...
        let proposal_hash = proposal.map(|p| p.hash());

        // ... and we broadcast it. The function will wait for 2f+1 precommits, aggregate them and
        // return.
        let precom_agg = self
            .deps
            .broadcast_and_aggregate(round, Step::Precommit, proposal_hash.clone())
            .await?;

        // We transform the aggregation we got into an actual vote result. See the function for more
        // details.
        let precommit = aggregation_to_vote(proposal_hash, precom_agg);

        // Match the vote result and update Tendermint's state.
        match precommit {
            VoteResult::Block(proof) => {
                // We only get the Block result if there were 2f+1 precommits for the proposal we
                // voted on (not Nil or another proposal). The proof is simply the aggregation of
                // those 2f+1 precommits (The signatures more specifically. But they are not just
                // signatures of the proposal hash, it's a bit more complicated. See the Handel
                // crate for more details.), we need it to assemble the block.
                self.state.current_proof = Some(proof);
                self.state.current_checkpoint = Checkpoint::OnDecision;
            }
            VoteResult::Nil(_) => {
                // We get the Nil result if there were 2f+1 prevotes for Nil. We change our state
                // to OnTimeoutPrecommit because the Tendermint protocol does not handle the Nil
                // case, instead it just let us timeout.
                self.state.current_checkpoint = Checkpoint::OnTimeoutPrecommit;
            }
            VoteResult::Timeout => {
                // In all other cases, we get the Timeout result.
                self.state.current_checkpoint = Checkpoint::OnTimeoutPrecommit;
            }
            VoteResult::NewRound(round) => {
                // If the network got f+1 messages for a round greater than our current one, we are
                // warned with the NewRound result and we start a new round.
                // This corresponds to lines 55-56 of Tendermint consensus algorithm.
                self.state.round = round;
                self.state.current_checkpoint = Checkpoint::StartRound;
            }
        }

        Ok(())
    }
}
