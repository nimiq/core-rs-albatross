use crate::state::TendermintState;
use crate::utils::{AggregationResult, ProposalResult, Step, TendermintError};
use async_trait::async_trait;
use beserial::{Deserialize, Serialize};
use nimiq_hash::Blake2sHash;
use std::sync::Arc;

#[async_trait]
pub trait TendermintOutsideDeps {
    type ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static;
    type ProofTy: Clone + Send + Sync + 'static;
    type ResultTy: Send + Sync + 'static;

    // The functions from the validator

    // TODO: What exactly is this supposed to verify???
    fn verify_state<ProposalTy, ProofTy>(
        &self,
        state: TendermintState<ProposalTy, ProofTy>,
    ) -> bool
    where
        ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
        ProofTy: Clone + Send + Sync + 'static;

    fn is_our_turn(&self, round: u32) -> bool;

    fn get_value(&self, round: u32) -> Self::ProposalTy;

    fn is_valid(&self, proposal: Arc<Self::ProposalTy>) -> bool;

    fn assemble_block(
        &self,
        proposal: Arc<Self::ProposalTy>,
        proof: Self::ProofTy,
    ) -> Self::ResultTy;

    // Future
    async fn broadcast_proposal(
        &self,
        round: u32,
        proposal: Arc<Self::ProposalTy>,
        valid_round: Option<u32>,
    );

    // Future
    async fn await_proposal(&self, round: u32) -> ProposalResult<Self::ProposalTy>;

    // The functions from Handel

    // Future
    async fn broadcast_and_aggregate(
        &self,
        round: u32,
        step: Step,
        proposal: Option<Blake2sHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError>;

    fn get_aggregation(&self, round: u32, step: Step) -> Option<AggregationResult<Self::ProofTy>>;

    fn cancel_aggregation(&self, round: u32, step: Step) -> Result<(), TendermintError>;
}
