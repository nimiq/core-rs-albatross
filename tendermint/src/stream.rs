use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::tendermint::Tendermint;
use crate::utils::{Checkpoint, TendermintError, TendermintReturn};
use async_stream::stream;
use futures::Stream;
use nimiq_hash::Hash;

/// This is the main function of the Tendermint crate. Calling this function returns a Stream that,
/// when called repeatedly, yields state updates, errors and results produced by the Tendermint
/// protocol.
/// You need to input some type that implements the TendermintOutsideDeps trait, this trait has all
/// the methods that Tendermint needs in order to interact with the network, produce proposals,
/// verify proposals, etc. This code only implements the high-level Tendermint protocol, so
/// TendermintOutsideDeps needs to provide all that low-level functionality.
/// Optionally, we can also input a TendermintState. This allows us to recover from a previous
/// state. The Stream is always sending the current state. If, for some reason, Tendermint gets
/// interrupted, we can resume from where we left off by calling `expect_block` with the last state
/// that we received.
/// Internally, our Tendermint code works like a state machine, moving from state to state until it
/// either returns a completed block or an error.
pub fn expect_block<DepsTy, ProposalTy, ProofTy, ResultTy>(
    // A type that implements TendermintOutsideDeps.
    deps: DepsTy,
    // An optional input for the TendermintState.
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
    // We check if a state was inputted. If yes (and it is valid), we initialize Tendermint with it.
    // If not, we create a new empty Tendermint.
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

    // This is the main loop of the function. It progresses the Tendermint state machine.
    loop {
        // We run the next transition given our current state. This returns a
        // Result<(), TendermintError>. Unless we get a completed block from on_decision(), in which
        // case we yield the block and terminate.
        let checkpoint_res = match tendermint.state.current_checkpoint {
            Checkpoint::StartRound => tendermint.start_round().await,
            Checkpoint::OnProposal => tendermint.on_proposal().await,
            Checkpoint::OnPastProposal => tendermint.on_past_proposal().await,
            Checkpoint::OnPolka => tendermint.on_polka().await,
            Checkpoint::OnNilPolka => tendermint.on_nil_polka().await,
            Checkpoint::OnDecision => match tendermint.on_decision() {
                Ok(block) => {
                    yield TendermintReturn::Result(block);
                    return;
                }
                Err(error) => Err(error),
            },
            Checkpoint::OnTimeoutPropose => tendermint.on_timeout_propose().await,
            Checkpoint::OnTimeoutPrevote => tendermint.on_timeout_prevote().await,
            Checkpoint::OnTimeoutPrecommit => tendermint.on_timeout_precommit(),
        };

        // If we got an error from the last state transition, we yield it and then terminate.
        if let Err(error) = checkpoint_res {
            yield TendermintReturn::Error(error);
            return;
        }

        // If we did not get an error, we yield our current state and loop again.
        yield TendermintReturn::StateUpdate(tendermint.state.clone());
    }
    }
}
