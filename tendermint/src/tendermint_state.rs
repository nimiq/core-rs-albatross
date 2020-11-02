use crate::{Step, TendermintReturn};
use beserial::{Deserialize, Serialize};
use futures::channel::mpsc::UnboundedSender;
use std::sync::Arc;

pub struct TendermintState<ProposalTy, ProofTy, ResultTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
    ResultTy: Send + Sync + 'static,
{
    pub round: u32,
    pub step: Step,
    pub locked_value: Option<Arc<ProposalTy>>,
    pub locked_round: Option<u32>,
    pub valid_value: Option<Arc<ProposalTy>>,
    pub valid_round: Option<u32>,
    pub current_proposal: Option<Arc<ProposalTy>>,
    pub current_proof: Option<ProofTy>,
    pub return_stream: Option<UnboundedSender<TendermintReturn<ProposalTy, ProofTy, ResultTy>>>,
}

impl<ProposalTy, ProofTy, ResultTy> TendermintState<ProposalTy, ProofTy, ResultTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
    ResultTy: Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            round: 0,
            step: Step::Propose,
            locked_value: None,
            locked_round: None,
            valid_value: None,
            valid_round: None,
            current_proposal: None,
            current_proof: None,
            return_stream: None,
        }
    }
}

pub struct TendermintStateProof<ProposalTy, ProofTy>
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
    pub current_proposal: Option<Arc<ProposalTy>>,
    pub current_proof: Option<ProofTy>,
}
