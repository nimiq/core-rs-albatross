use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::tendermint::Tendermint;
use crate::utils::{Checkpoint, TendermintError, TendermintReturn};
use async_stream::stream;
use futures::Stream;
use nimiq_hash::Hash;

pub fn expect_block<DepsTy, ProposalTy, ProofTy, ResultTy>(
    deps: DepsTy,
    state_opt: Option<TendermintState<ProposalTy, ProofTy>>,
) -> impl Stream<Item = TendermintReturn<ProposalTy, ProofTy, ResultTy>>
where
    ProposalTy: Clone + PartialEq + Hash + Unpin + 'static,
    ProofTy: Clone + Unpin + 'static,
    ResultTy: Unpin + 'static,
    DepsTy: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
        + 'static,
{
    stream! {
     let mut tendermint = if let Some(state) = state_opt {
         if deps.verify_state(&state) {
             Tendermint { deps, state }
         } else {
             yield TendermintReturn::Error(TendermintError::BadInitState);
             return;
         }
     } else {
         Tendermint::new(deps)
     };

     loop {
         match tendermint.state.current_checkpoint {
             Checkpoint::StartRound => tendermint.start_round().await,
             Checkpoint::OnProposal => tendermint.on_proposal().await,
             Checkpoint::OnPastProposal => tendermint.on_past_proposal().await,
             Checkpoint::OnPolka => tendermint.on_polka().await,
             Checkpoint::OnNilPolka => tendermint.on_nil_polka().await,
             Checkpoint::OnDecision => {
                 let block = tendermint.on_decision();
                 yield TendermintReturn::Result(block);
                 return;
             }
             Checkpoint::OnTimeoutPropose => tendermint.on_timeout_propose().await,
             Checkpoint::OnTimeoutPrevote => tendermint.on_timeout_prevote().await,
             Checkpoint::OnTimeoutPrecommit => tendermint.on_timeout_precommit(),
         }

         yield TendermintReturn::StateUpdate(tendermint.state.clone());
     }
    }
}
