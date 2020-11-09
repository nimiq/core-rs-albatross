use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::utils::Checkpoint;
use nimiq_hash::Hash;
use std::clone::Clone;

pub struct Tendermint<
    ProposalTy: Clone + PartialEq + Hash + Unpin + 'static,
    ProofTy: Clone + Unpin + 'static,
    ResultTy: Unpin + 'static,
    DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
        + 'static,
> {
    pub(crate) deps: DepsTy,
    pub(crate) state: TendermintState<ProposalTy, ProofTy>,
}

impl<
        ProposalTy: Clone + PartialEq + Hash + Unpin + 'static,
        ProofTy: Clone + Unpin + 'static,
        ResultTy: Unpin + 'static,
        DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
            + 'static,
    > Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy>
{
    pub(crate) fn new(deps: DepsTy) -> Tendermint<ProposalTy, ProofTy, ResultTy, DepsTy> {
        Self {
            deps,
            state: TendermintState::new(),
        }
    }
}
