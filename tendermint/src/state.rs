use crate::utils::{Checkpoint, Step};
use beserial::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct TendermintState<ProposalTy, ProofTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
{
    pub round: u32,
    pub step: Step,
    pub locked_value: Option<Arc<ProposalTy>>,
    pub locked_round: Option<u32>,
    pub valid_value: Option<Arc<ProposalTy>>,
    pub valid_round: Option<u32>,
    pub current_checkpoint: Checkpoint,
    pub current_proposal: Option<Arc<ProposalTy>>,
    pub current_proposal_vr: Option<u32>,
    pub current_proof: Option<ProofTy>,
}

impl<ProposalTy, ProofTy> TendermintState<ProposalTy, ProofTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            round: 0,
            step: Step::Propose,
            locked_value: None,
            locked_round: None,
            valid_value: None,
            valid_round: None,
            current_checkpoint: Checkpoint::StartRound,
            current_proposal: None,
            current_proposal_vr: None,
            current_proof: None,
        }
    }
}
