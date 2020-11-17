use crate::state::TendermintState;
use crate::utils::{AggregationResult, ProposalResult, Step, TendermintError};
use crate::{ProofTrait, ProposalTrait, ResultTrait};
use async_trait::async_trait;
use nimiq_hash::Blake2sHash;

/// The (async) trait that we need for all of Tendermint's low-level functions. The functions are
/// mostly about producing proposals and networking.
#[async_trait]
pub trait TendermintOutsideDeps {
    type ProposalTy: ProposalTrait;
    type ProofTy: ProofTrait;
    type ResultTy: ResultTrait;

    /// Verify that a given Tendermint state is valid. This is necessary when we are initializing
    /// using a previous state.
    fn verify_state(&self, state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool;

    /// Checks if it our turn to propose for the given round.
    fn is_our_turn(&self, round: u32) -> bool;

    /// Produces a proposal for the given round. It is used when it is our turn to propose. The
    /// proposal is guaranteed to be valid.
    fn get_value(&self, round: u32) -> Result<Self::ProposalTy, TendermintError>;

    /// Takes a proposal and a proof (2f+1 precommits) and returns a completed block.
    fn assemble_block(
        &self,
        proposal: Self::ProposalTy,
        proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError>;

    /// Broadcasts a proposal message (which includes the proposal and the proposer's valid round).
    /// This is a Future and it is allowed to fail.
    async fn broadcast_proposal(
        &self,
        round: u32,
        proposal: Self::ProposalTy,
        valid_round: Option<u32>,
    ) -> Result<(), TendermintError>;

    /// Waits for a proposal message (which includes the proposal and the proposer's valid round).
    /// The received proposal (if any) is guaranteed to be valid. This function also has to take
    /// care of waiting before timing out.
    /// This is a Future and it is allowed to fail.
    async fn await_proposal(
        &self,
        round: u32,
    ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError>;

    /// Broadcasts a vote (either prevote or precommit) for a given round and proposal. It then
    /// returns an aggregation of the 2f+1 votes received from other nodes for this round (and
    /// corresponding step).
    /// It also has to take care of waiting before timing out.
    /// This is a Future and it is allowed to fail.
    async fn broadcast_and_aggregate(
        &mut self,
        round: u32,
        step: Step,
        proposal: Option<Blake2sHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError>;

    /// Returns the current aggregation for a given round and step. The returned aggregation might
    /// or not have 2f+1 votes, this function only returns all the votes that we have so far.
    /// It will fail if no aggregation was started for the given round and step.
    /// This is a Future and it is allowed to fail.
    async fn get_aggregation(
        &self,
        round: u32,
        step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError>;

    /// Cancels the current aggregation for a given round and step.
    /// It will fail if no aggregation was started for the given round and step.
    fn cancel_aggregation(&mut self, round: u32, step: Step) -> Result<(), TendermintError>;
}
