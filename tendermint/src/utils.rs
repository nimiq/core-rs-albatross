use crate::state::TendermintState;
use crate::ProofTrait;
use nimiq_hash::Blake2sHash;
use nimiq_primitives::policy::TWO_THIRD_SLOTS;
use std::collections::BTreeMap;

/// Represents the current stage of the Tendermint state machine. Each stage corresponds to a
/// function of our implementation of the Tendermint protocol (see protocol.rs).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Checkpoint {
    StartRound,
    OnProposal,
    OnPastProposal,
    OnPolka,
    OnNilPolka,
    OnDecision,
    OnTimeoutPropose,
    OnTimeoutPrevote,
    OnTimeoutPrecommit,
}

/// The steps of the Tendermint protocol, as described in the paper.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Step {
    Propose,
    Prevote,
    Precommit,
}

/// Used to represent our vote decision (prevote or precommit) on a given proposal.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VoteDecision {
    Block,
    Nil,
}

/// Represents the results we can get when waiting for a proposal message.
#[derive(Clone, Debug)]
pub enum ProposalResult<ProposalTy> {
    // Means we have received a proposal message. The first field is the actual proposal, the second
    // one is the valid round of the proposer (being None is equal to the -1 used in the protocol).
    Proposal(ProposalTy, Option<u32>),
    // Means that we have timed out while waiting for the proposal message.
    Timeout,
}

/// Represents the results we can get when waiting for a vote (either prevote or precommit).
#[derive(Clone, Debug)]
pub enum VoteResult<ProofTy> {
    // Means that we have received 2f+1 votes for a block. The field is the aggregation of the votes
    // signatures (it's a bit more complicated than this, see the Handel crate for more details).
    Block(ProofTy),
    // Means that we have received 2f+1 votes for Nil. The field is the aggregation of the votes
    // signatures (it's a bit more complicated than this, see the Handel crate for more details).
    Nil(ProofTy),
    // Means that we have timed out while waiting for the votes.
    Timeout,
    // Means that we have received f+1 messages for a round greater than our current one. The field
    // states for which round we received those messages.
    NewRound(u32),
}

/// Represents the results we can get from calling `broadcast_and_aggregate` from
/// TendermintOutsideDeps.
#[derive(Clone, Debug)]
pub enum AggregationResult<ProofTy> {
    // If the aggregation was able to complete (get 2f+1 votes), we receive a BTreeMap of the
    // different vote messages (Some(hash) is a vote for a proposal with that hash, None is a vote
    // for Nil) along with the corresponding proofs (the aggregation of the votes signatures) and
    // a integer representing how many votes were received for that particular message.
    Aggregation(BTreeMap<Option<Blake2sHash>, (ProofTy, usize)>),
    // Means that we have received f+1 messages for a round greater than our current one. The field
    // states for which round we received those messages.
    NewRound(u32),
}

/// These are the possible return options for the `expect_block` Stream.
#[derive(Clone, Debug)]
pub enum TendermintReturn<ProposalTy, ProofTy, ResultTy> {
    // Means we got a completed block. The field is the block.
    Result(ResultTy),
    // This just sends our current state. It is useful in case we go down for some reason and need
    // to start from the point where we left off.
    StateUpdate(TendermintState<ProposalTy, ProofTy>),
    // Just means that we encountered an error.
    Error(TendermintError),
}

/// An enum containing possible errors that can happen to Tendermint.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TendermintError {
    BadInitState,
    AggregationError,
    AggregationDoesNotExist,
    ProposalBroadcastError,
    CannotReceiveProposal,
    CannotProduceProposal,
    CannotAssembleBlock,
}

/// An utility function that converts an AggregationResult into a VoteResult. The AggregationResult
/// just returns the raw vote messages it got (albeit in an aggregated form), so we need provide the
/// semantics to translate that into the VoteResult that the Tendermint protocol expects.
pub(crate) fn aggregation_to_vote<ProofTy: ProofTrait>(
    // This is the hash of the proposal we voted for (None means we voted for Nil).
    proposal: Option<Blake2sHash>,
    aggregation: AggregationResult<ProofTy>,
) -> VoteResult<ProofTy> {
    match aggregation {
        // If we got an aggregation we need to handle it.
        AggregationResult::Aggregation(agg) => {
            if proposal.is_some()
                && agg.get(&proposal).map_or(0, |x| x.1) >= TWO_THIRD_SLOTS as usize
            {
                // If we received 2f+1 votes for the same proposal that we voted on (assuming that
                // we didn't vote Nil), then we must return Block.
                VoteResult::Block(agg.get(&proposal).cloned().unwrap().0)
            } else if agg.get(&None).map_or(0, |x| x.1) >= TWO_THIRD_SLOTS as usize {
                // If we received 2f+1 votes for Nil (irrespective of us voting for a block or not),
                // then we must return Nil.
                VoteResult::Nil(agg.get(&None).cloned().unwrap().0)
            } else {
                // There are two cases when we must return Timeout:
                // 1) When we receive 2f+1 votes for a proposal for which we didn't vote (so we
                //    voted Nil or for a different proposal).
                // 2) When we don't have 2f+1 votes for a single proposal or for Nil.
                VoteResult::Timeout
            }
        }
        // If we got f+1 votes for a round greater than our current one, we must return NewRound.
        AggregationResult::NewRound(round) => VoteResult::NewRound(round),
    }
}

/// An utility function that checks if a given AggregationResult has 2f+1 votes for a given
/// proposal.
pub(crate) fn has_2f1_votes<ProofTy: ProofTrait>(
    proposal: Blake2sHash,
    aggregation: AggregationResult<ProofTy>,
) -> bool {
    let agg = match aggregation {
        AggregationResult::Aggregation(v) => v,
        AggregationResult::NewRound(_) => return false,
    };

    agg.get(&Some(proposal)).map_or(0, |x| x.1) >= TWO_THIRD_SLOTS as usize
}
