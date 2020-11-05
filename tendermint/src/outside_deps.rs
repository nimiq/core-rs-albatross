use crate::utils::{AggregationResult, ProposalResult, Step, TendermintError, VoteDecision};
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

    fn is_our_turn(&self, round: u32) -> bool;

    fn is_valid(&self, proposal: Arc<Self::ProposalTy>) -> bool;

    fn get_value(&self, round: u32) -> Self::ProposalTy;

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

    // TODO: Does it need to be a Result??
    fn cancel_aggregation(&self, round: u32, step: Step) -> Result<(), TendermintError>;

    // Not sure where this is from.

    // TODO: What exactly is this supposed to verify???
    fn verify_proposal_state(&self, round: u32, previous_precommit_result: &Self::ProofTy) -> bool;

    // TODO: What exactly is this supposed to verify???
    fn verify_prevote_state(
        &self,
        round: u32,
        proposal: Arc<Self::ProposalTy>,
        previous_precommit_result: &Self::ProofTy,
    ) -> bool;

    // TODO: What exactly is this supposed to verify???
    fn verify_precommit_state(
        &self,
        round: u32,
        proposal: Option<Arc<Self::ProposalTy>>,
        prevote_result: &Self::ProofTy,
    ) -> Option<VoteDecision>;
}
