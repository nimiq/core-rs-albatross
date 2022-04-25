use crate::state::TendermintState;
use crate::{
    ProofTrait, ProposalCacheTrait, ProposalHashTrait, ProposalTrait, TendermintOutsideDeps,
};
use beserial::{Deserialize, Serialize};
use nimiq_block::TendermintStep;
use nimiq_primitives::policy::TWO_F_PLUS_ONE;
use std::collections::BTreeMap;
use thiserror::Error;

/// Represents the current stage of the Tendermint state machine. Each stage corresponds to a
/// function of our implementation of the Tendermint protocol (see protocol.rs).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[repr(u8)]
pub enum Checkpoint {
    StartRound,
    Propose,
    WaitForProposal,
    VerifyValidRound,
    AggregatePreVote,
    AggregatePreCommit,
}

/// The steps of the Tendermint protocol, as described in the paper.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[repr(u8)]
pub enum Step {
    Propose,
    Prevote,
    Precommit,
}

/// A method for easy conversion of Step into TendermintStep.
impl From<Step> for TendermintStep {
    fn from(step: Step) -> Self {
        match step {
            Step::Precommit => TendermintStep::PreCommit,
            Step::Prevote => TendermintStep::PreVote,
            Step::Propose => TendermintStep::Propose,
        }
    }
}

/// Used to represent our vote decision (prevote or precommit) on a given proposal.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VoteDecision {
    Block,
    Nil,
}

/// Represents the results we can get when waiting for a proposal message.
#[derive(Clone, Debug)]
pub enum ProposalResult<ProposalTy: ProposalTrait, ProposalCacheTy: ProposalCacheTrait> {
    // Means we have received a proposal message. The first field is the actual proposal, the second
    // one is the valid round of the proposer (being None is equal to the -1 used in the protocol).
    Proposal((ProposalTy, ProposalCacheTy), Option<u32>),
    // Means that we have timed out while waiting for the proposal message.
    Timeout,
}

/// Represents the results we can get when waiting for a vote (either prevote or precommit).
#[derive(Clone, Debug)]
pub enum VoteResult<ProofTy: ProofTrait> {
    // Means that we have received 2f+1 votes for a block. The field is the aggregation of the votes
    // signatures (it's a bit more complicated than this, see the Handel crate for more details).
    Block(ProofTy),
    // Means that we have received 2f+1 votes for Nil. The field is the aggregation of the votes
    // signatures (it's a bit more complicated than this, see the Handel crate for more details).
    Nil(ProofTy),
    // Means that we have timed out while waiting for the votes.
    Timeout,
}

/// Represents the results we can get from calling `broadcast_and_aggregate` from
/// TendermintOutsideDeps.
#[derive(Clone, Debug)]
pub enum AggregationResult<ProposalHashTy: ProposalHashTrait, ProofTy: ProofTrait> {
    // If the aggregation was able to complete (get 2f+1 votes), we receive a BTreeMap of the
    // different vote messages (Some(hash) is a vote for a proposal with that hash, None is a vote
    // for Nil) along with the corresponding proofs (the aggregation of the votes signatures) and
    // a integer representing how many votes were received for that particular message.
    Aggregation(BTreeMap<Option<ProposalHashTy>, (ProofTy, u16)>),
    // Means that we have received f+1 messages for a round greater than our current one. The field
    // states for which round we received those messages.
    NewRound(u32),
}

/// These are the possible return options for the `expect_block` Stream.
#[derive(Clone, Debug)]
pub enum TendermintReturn<DepsTy: TendermintOutsideDeps> {
    // Means we got a completed block. The field is the block.
    Result(DepsTy::ResultTy),
    // This just sends our current state. It is useful in case we go down for some reason and need
    // to start from the point where we left off.
    StateUpdate(
        TendermintState<
            DepsTy::ProposalTy,
            DepsTy::ProposalCacheTy,
            DepsTy::ProposalHashTy,
            DepsTy::ProofTy,
        >,
    ),
    // Just means that we encountered an error.
    Error(TendermintError),
}

/// An enum containing possible errors that can happen to Tendermint.
#[derive(Error, Debug, Clone)]
pub enum TendermintError {
    #[error("Handel aggregation failed.")]
    AggregationError,
    #[error("Handel aggregation does not exist.")]
    AggregationDoesNotExist,
    #[error("Broadcasting the proposal failed.")]
    ProposalBroadcastError,
    #[error("Could not receive a proposal.")]
    CannotReceiveProposal,
    #[error("Could not produce a proposal.")]
    CannotProduceProposal,
    #[error("Could not assemble a finalized block.")]
    CannotAssembleBlock,
}

pub enum StreamResult<DepsTy: TendermintOutsideDeps> {
    Tendermint(TendermintReturn<DepsTy>),
    BackgroundTask,
}

/// An utility function that converts an AggregationResult into a VoteResult. The AggregationResult
/// just returns the raw vote messages it got (albeit in an aggregated form), so we need provide the
/// semantics to translate that into the VoteResult that the Tendermint protocol expects.
pub(crate) fn aggregation_to_vote<ProposalHashTy: ProposalHashTrait, ProofTy: ProofTrait>(
    // This is the hash of the current proposal (None means we don't have a current proposal).
    proposal_hash: &Option<ProposalHashTy>,
    agg: BTreeMap<Option<ProposalHashTy>, (ProofTy, u16)>,
) -> VoteResult<ProofTy> {
    if proposal_hash.is_some() && agg.get(proposal_hash).map_or(0, |x| x.1) >= TWO_F_PLUS_ONE {
        log::debug!(
            "Current proposal {:?} has {} votes: {:#?}",
            &proposal_hash,
            agg.get(proposal_hash).map_or(0, |x| x.1),
            &agg,
        );
        // If we received 2f+1 votes for the current (assuming that it isn't None), then we
        // must return Block.
        VoteResult::Block(agg.get(proposal_hash).cloned().unwrap().0)
    } else if agg.get(&None).map_or(0, |x| x.1) >= TWO_F_PLUS_ONE {
        // is f+1 sufficient here?
        log::debug!(
            "Nil has {} votes: {:#?}",
            agg.get(&None).map_or(0, |x| x.1),
            &agg,
        );
        // If we received 2f+1 votes for Nil, then we must return Nil.
        VoteResult::Nil(agg.get(&None).cloned().unwrap().0)
    } else {
        log::debug!("Aggregation timed out: {:#?}", &agg);
        // There are two cases when we must return Timeout:
        // 1) When we receive 2f+1 votes for a proposal that we don't have.
        // 2) When we don't have 2f+1 votes for a single proposal or for Nil.
        VoteResult::Timeout
    }
}

/// An utility function that checks if a given AggregationResult has 2f+1 votes for a given
/// proposal.
pub(crate) fn has_2f1_votes<ProposalHashTy: ProposalHashTrait, ProofTy: ProofTrait>(
    proposal: ProposalHashTy,
    aggregation: AggregationResult<ProposalHashTy, ProofTy>,
) -> bool {
    let agg = match aggregation {
        AggregationResult::Aggregation(v) => v,
        AggregationResult::NewRound(_) => {
            log::debug!("Checking proposal for vr returned NewRound (should be unreachable!())");
            return false;
        }
    };
    let prop_opt = Some(proposal);
    log::debug!(
        "Vr proposal {:?} has {} votes",
        &prop_opt,
        agg.get(&prop_opt).map_or(0, |x| x.1),
    );
    agg.get(&prop_opt).map_or(0, |x| x.1) >= TWO_F_PLUS_ONE
}
