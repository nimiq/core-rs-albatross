use std::marker::PhantomData;
use std::task::Poll;

use crate::outside_deps::TendermintOutsideDeps;
use crate::state::TendermintState;
use crate::tendermint::Tendermint;
use crate::utils::{StreamResult, TendermintReturn};
use futures::{
    future::FutureExt,
    stream::{BoxStream, SelectAll, Stream, StreamExt},
};

/// This is the main struct of the Tendermint crate. It implements Stream which,
/// when called repeatedly, yields state updates, errors and results produced by the Tendermint
/// protocol.
/// You need to input some type that implements the TendermintOutsideDeps trait, this trait has all
/// the methods that Tendermint needs in order to interact with the network, produce proposals,
/// etc. This code only implements the high-level Tendermint protocol, so TendermintOutsideDeps
/// needs to provide all that low-level functionality.
/// Optionally, we can also input a TendermintState. This allows us to recover from a previous
/// state. The Stream is always sending the current state. If, for some reason, Tendermint gets
/// interrupted, we can resume from where we left off by creating a new instance with the last state
/// that we received.
/// Internally, our Tendermint code works like a state machine, moving from state to state until it
/// either returns a completed block or an error
pub struct TendermintStreamWrapper<DepsTy: TendermintOutsideDeps> {
    tendermint_stream: BoxStream<'static, StreamResult<DepsTy>>,
    phantom: PhantomData<DepsTy>,
}

impl<DepsTy: TendermintOutsideDeps> TendermintStreamWrapper<DepsTy> {
    pub fn new(
        // A type that implements TendermintOutsideDeps.
        mut deps: DepsTy,
        // An optional input for the TendermintState.
        state_opt: Option<
            TendermintState<
                DepsTy::ProposalTy,
                DepsTy::ProposalCacheTy,
                DepsTy::ProposalHashTy,
                DepsTy::ProofTy,
            >,
        >,
    ) -> Result<Self, DepsTy> {
        debug!(
            "Initializing tendermint with{} prior state",
            if state_opt.is_some() { "" } else { "out" }
        );

        if let Some(state) = &state_opt {
            if !deps.verify_state(state) {
                return Err(deps);
            }
        }

        // get the background_stream from the outside deps.
        let background_task = deps
            .get_background_task()
            .into_stream()
            .map(|_| StreamResult::BackgroundTask)
            .boxed();

        // also get the actual tendermint stream
        let tendermint_stream = Tendermint::new(deps, state_opt)
            .map(StreamResult::Tendermint)
            .boxed();

        // put them both in a select such that both of them are driven
        let mut select = Box::pin(SelectAll::new());
        select.push(tendermint_stream);
        select.push(background_task);

        Ok(Self {
            tendermint_stream: select,
            phantom: PhantomData,
        })
    }
}

impl<DepsTy: TendermintOutsideDeps> Stream for TendermintStreamWrapper<DepsTy> {
    type Item = TendermintReturn<DepsTy>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(result) = self.tendermint_stream.poll_next_unpin(cx) {
            match result {
                None => return Poll::Ready(None),
                Some(StreamResult::Tendermint(result)) => return Poll::Ready(Some(result)),
                Some(StreamResult::BackgroundTask) => {}
            }
        }
        Poll::Pending
    }
}
