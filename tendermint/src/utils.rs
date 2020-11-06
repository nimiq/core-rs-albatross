use crate::state::TendermintState;
use beserial::{Deserialize, Serialize};
use nimiq_hash::Blake2sHash;
use nimiq_primitives::policy::TWO_THIRD_SLOTS;
use std::collections::BTreeMap;

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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Step {
    Propose,
    Prevote,
    Precommit,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VoteDecision {
    Block,
    Nil,
}

pub enum ProposalResult<ProposalTy> {
    // value + valid round
    Proposal(ProposalTy, Option<u32>),
    Timeout,
}

pub enum VoteResult<ProofTy> {
    Block(ProofTy),
    Nil(ProofTy),
    Timeout,
    NewRound(u32),
}

pub enum AggregationResult<ProofTy> {
    Aggregation(BTreeMap<Option<Blake2sHash>, (ProofTy, usize)>),
    NewRound(u32),
}

pub enum TendermintReturn<ProposalTy, ProofTy, ResultTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
{
    Result(ResultTy),
    StateUpdate(TendermintState<ProposalTy, ProofTy>),
    Error(TendermintError),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TendermintError {
    BadInitState,
    AggregationError,
}

pub(crate) fn aggregation_to_vote<ProofTy: Clone>(
    proposal: Option<Blake2sHash>,
    aggregation: AggregationResult<ProofTy>,
) -> VoteResult<ProofTy> {
    match aggregation {
        AggregationResult::Aggregation(agg) => {
            if proposal.is_some()
                && agg.get(&proposal).map_or(0, |x| x.1) >= TWO_THIRD_SLOTS as usize
            {
                VoteResult::Block(agg.get(&proposal).cloned().unwrap().0)
            } else if agg.get(&None).map_or(0, |x| x.1) >= TWO_THIRD_SLOTS as usize {
                VoteResult::Nil(agg.get(&None).cloned().unwrap().0)
            } else {
                VoteResult::Timeout
            }
        }
        AggregationResult::NewRound(round) => VoteResult::NewRound(round),
    }
}
