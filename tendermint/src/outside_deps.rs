use crate::state::TendermintState;
use crate::utils::{AggregationResult, ProposalResult, Step, TendermintError};
use async_trait::async_trait;
use nimiq_hash::{Blake2sHash, Hash};

#[async_trait]
pub trait TendermintOutsideDeps {
    type ProposalTy: Clone + PartialEq + Hash + Unpin + 'static;
    type ProofTy: Clone + Unpin + 'static;
    type ResultTy: Unpin + 'static;

    // The functions from the validator

    // TODO: What exactly is this supposed to verify???
    fn verify_state(&self, state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool;

    fn is_our_turn(&self, round: u32) -> bool;

    fn is_valid(&self, proposal: Self::ProposalTy) -> bool;

    fn get_value(&self, round: u32) -> Result<Self::ProposalTy, TendermintError>;

    fn assemble_block(
        &self,
        proposal: Self::ProposalTy,
        proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError>;

    // Future
    async fn broadcast_proposal(
        &self,
        round: u32,
        proposal: Self::ProposalTy,
        valid_round: Option<u32>,
    ) -> Result<(), TendermintError>;

    // Future
    async fn await_proposal(
        &self,
        round: u32,
    ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError>;

    // The functions from Handel

    // Future
    async fn broadcast_and_aggregate(
        &self,
        round: u32,
        step: Step,
        proposal: Option<Blake2sHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError>;

    fn get_aggregation(
        &self,
        round: u32,
        step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError>;

    fn cancel_aggregation(&self, round: u32, step: Step) -> Result<(), TendermintError>;
}
