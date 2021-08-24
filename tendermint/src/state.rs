use crate::{
    utils::{Checkpoint, Step},
    ProofTrait, ProposalTrait,
};

/// The struct that stores Tendermint's state. It contains both the state variables describes in the
/// protocol (except for the height and the decision vector since we don't need them) and some extra
/// variables that we need for our implementation (those are prefixed with "current").
#[derive(Clone, Debug)]
pub struct TendermintState<ProposalTy: ProposalTrait, ProofTy: ProofTrait> {
    pub round: u32,
    pub step: Step,
    pub locked_value: Option<ProposalTy>,
    pub locked_round: Option<u32>,
    pub valid_value: Option<ProposalTy>,
    pub valid_round: Option<u32>,
    pub current_checkpoint: Checkpoint,
    pub current_proposal: Option<ProposalTy>,
    pub current_proposal_vr: Option<u32>,
    pub current_proof: Option<ProofTy>,
}

impl<ProposalTy: ProposalTrait, ProofTy: ProofTrait> TendermintState<ProposalTy, ProofTy> {
    pub fn new(initial_round: u32) -> Self {
        Self {
            round: initial_round,
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
