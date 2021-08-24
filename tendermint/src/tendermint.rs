use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::{ProofTrait, ProposalTrait, ResultTrait};

/// This is the struct that implements the Tendermint state machine. Its only fields are deps
/// (dependencies, any type that implements the trait TendermintOutsideDeps, needed for a variety of
/// low-level tasks) and state (stores the current state of Tendermint).
pub struct Tendermint<
    ProposalTy: ProposalTrait,
    ProofTy: ProofTrait,
    ResultTy: ResultTrait,
    DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
        + 'static,
> {
    pub deps: DepsTy,
    pub state: TendermintState<ProposalTy, ProofTy>,
}
