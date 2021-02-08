use crate::outside_deps::TendermintOutsideDeps;
use crate::tendermint::Tendermint;
use crate::utils::{Checkpoint, Step, VoteDecision};
use crate::TendermintError;
use crate::{ProofTrait, ProposalTrait, ResultTrait};

/// This section has methods that implement the Tendermint protocol as described in the original
/// paper (https://arxiv.org/abs/1807.04938v3).
/// We made three modifications:
/// 1) For each round, we only accept the first proposal that we receive. Since Tendermint is
///    designed to work even if the proposer sends different proposals to each node, and the nodes
///    don't rebroadcast the proposals they receive, we can safely ignore any subsequent proposals
///    that we receive after the first one.
/// 2) We only accept valid proposals to begin with, so we don't need to check the validity of the
///    proposal during the protocol.
/// 3) The protocol assumes that we are always listening for proposals and precommit messages, and
///    if we receive any proposal with 2f+1 precommits accompanying it, we are supposed to accept it
///    and terminate. We don't do this. Instead we assume that the validator crate will be listening
///    for completed blocks and will terminate Tendermint when it receives one.
/// These two modifications allows us to refactor the Tendermint protocol from its original message
/// passing form into the state machine form that we use.
impl<
        ProposalTy: ProposalTrait,
        ProofTy: ProofTrait,
        ResultTy: ResultTrait,
        DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy> + 'static,
    > Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy>
{
    /// Lines 11-21 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) async fn start_round(&mut self) -> Result<(), TendermintError> {
        self.state.current_proposal = None;
        self.state.current_proposal_vr = None;
        self.state.current_proof = None;

        self.state.step = Step::Propose;

        let round = self.state.round;
        let valid_round = self.state.valid_round;

        if self.deps.is_our_turn(round) {
            let proposal = if self.state.valid_value.is_some() {
                self.state.valid_value.clone().unwrap()
            } else {
                self.deps.get_value(round)?
            };

            // Update our state and broadcast our proposal.
            self.state.current_proposal = Some(proposal.clone());
            self.state.current_proposal_vr = valid_round;
            self.deps.broadcast_proposal(round, proposal, valid_round).await?;

            // Prevote for our own proposal.
            self.state.step = Step::Prevote;
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block).await?;
        } else {
            self.await_proposal(round).await?;
        }

        Ok(())
    }

    /// Lines 22-27 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) async fn on_proposal(&mut self) -> Result<(), TendermintError> {
        assert_eq!(self.state.step, Step::Propose);

        self.state.step = Step::Prevote;

        let round = self.state.round;

        if self.state.locked_round.is_none() || self.state.locked_value == self.state.current_proposal {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block).await?;
        } else {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil).await?;
        }

        Ok(())
    }

    /// Lines 28-33 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) async fn on_past_proposal(&mut self) -> Result<(), TendermintError> {
        assert_eq!(self.state.step, Step::Propose);

        self.state.step = Step::Prevote;

        let round = self.state.round;

        if self.state.locked_round.unwrap_or(0) <= self.state.current_proposal_vr.unwrap() || self.state.locked_value == self.state.current_proposal {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Block).await?;
        } else {
            self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil).await?;
        }

        Ok(())
    }

    /// Lines 34-35 of Tendermint consensus algorithm (Algorithm 1)
    /// The `broadcast_and_aggregate_prevote` function takes care of waiting `timeoutPrevote`
    /// before returning.

    /// Lines 36-43 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) async fn on_polka(&mut self) -> Result<(), TendermintError> {
        // The protocol dictates that the step must be either prevote or precommit. But since in our
        // code it is impossible for this function to be called in the precommit step, we force the
        // step to be prevote.
        assert_eq!(self.state.step, Step::Prevote);

        let round = self.state.round;

        self.state.valid_value = self.state.current_proposal.clone();
        self.state.valid_round = Some(round);

        self.state.locked_value = self.state.current_proposal.clone();
        self.state.locked_round = Some(round);

        self.state.step = Step::Precommit;

        self.broadcast_and_aggregate_precommit(round, VoteDecision::Block).await?;

        Ok(())
    }

    /// Lines 44-46 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) async fn on_nil_polka(&mut self) -> Result<(), TendermintError> {
        assert_eq!(self.state.step, Step::Prevote);

        self.state.step = Step::Precommit;

        let round = self.state.round;

        self.broadcast_and_aggregate_precommit(round, VoteDecision::Nil).await?;

        Ok(())
    }

    /// Lines 47-48 of Tendermint consensus algorithm (Algorithm 1)
    /// The `broadcast_and_aggregate_precommit` function takes care of waiting `timeoutPrecommit`
    /// before returning.

    /// Lines 49-54 of Tendermint consensus algorithm (Algorithm 1)
    /// As stated before, we have the validator crate always listening for completed blocks. So this
    /// function only handle precommits for our current round and current proposal. The purpose of
    /// this function is then just to assemble the block if we can.
    pub(crate) fn on_decision(&mut self) -> Result<ResultTy, TendermintError> {
        self.deps.assemble_block(
            self.state.round,
            self.state.current_proposal.clone().unwrap(),
            self.state.current_proof.clone().unwrap(),
        )
    }

    /// Lines 55-56 of Tendermint consensus algorithm (Algorithm 1)
    /// We handle this situation on the functions `broadcast_and_aggregate_prevote` and
    /// `broadcast_and_aggregate_precommit`.

    /// Lines 57-60 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) async fn on_timeout_propose(&mut self) -> Result<(), TendermintError> {
        assert_eq!(self.state.step, Step::Propose);
        log::debug!("Proposal for round {} timed out.", self.state.round);

        self.state.step = Step::Prevote;

        let round = self.state.round;

        self.broadcast_and_aggregate_prevote(round, VoteDecision::Nil).await?;

        Ok(())
    }

    /// Lines 61-64 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) async fn on_timeout_prevote(&mut self) -> Result<(), TendermintError> {
        assert_eq!(self.state.step, Step::Prevote);

        self.state.step = Step::Precommit;

        let round = self.state.round;

        self.broadcast_and_aggregate_precommit(round, VoteDecision::Nil).await?;

        Ok(())
    }

    /// Lines 65-67 of Tendermint consensus algorithm (Algorithm 1)
    pub(crate) fn on_timeout_precommit(&mut self) -> Result<(), TendermintError> {
        self.state.round += 1;

        self.state.current_checkpoint = Checkpoint::StartRound;

        Ok(())
    }
}
