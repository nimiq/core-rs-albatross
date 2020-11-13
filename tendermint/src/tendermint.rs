use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use nimiq_hash::Hash;
use std::clone::Clone;

/// This is the struct that implements the Tendermint state machine. Its only fields are deps
/// (dependencies, any type that implements the trait TendermintOutsideDeps, needed for a variety of
/// low-level tasks) and state (stores the current state of Tendermint).
pub struct Tendermint<
    ProposalTy: Clone + PartialEq + Hash + Unpin + 'static,
    ProofTy: Clone + Unpin + 'static,
    ResultTy: Unpin + 'static,
    DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
        + 'static,
> {
    pub deps: DepsTy,
    pub state: TendermintState<ProposalTy, ProofTy>,
}

impl<
        ProposalTy: Clone + PartialEq + Hash + Unpin + 'static,
        ProofTy: Clone + Unpin + 'static,
        ResultTy: Unpin + 'static,
        DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
            + 'static,
    > Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy>
{
    /// Creates a new Tendermint state machine with an "empty" state.
    pub(crate) fn new(deps: DepsTy) -> Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy> {
        Self {
            deps,
            state: TendermintState::new(),
        }
    }
}
