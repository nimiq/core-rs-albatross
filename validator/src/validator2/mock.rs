use std::sync::Arc;

use failure::_core::pin::Pin;
use futures::channel::mpsc;
use futures::task::{Context, Poll};
use futures::{executor, Future, SinkExt, Stream};

use block_albatross::MacroBlock;
use blockchain_albatross::Blockchain;
use parking_lot::Mutex;
use utils::observer::Notifier;

pub struct Tendermint;
impl Tendermint {
    pub fn new() -> Self {
        Self {}
    }
}
impl Future for Tendermint {
    type Output = Result<MacroBlock, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }
}

pub struct ViewChangeHandel;
impl ViewChangeHandel {
    pub fn new(
        view_change: SignedViewChange,
        validator_id: u16,
        active_validator: ValidatorSlots,
    ) -> Self {
        unimplemented!()
    }
}
impl Future for ViewChangeHandel {
    type Output = ViewChangeProof;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }
}

pub fn notifier_to_stream<E: Clone + Send + Sync + 'static>(
    notifier: &mut Notifier<E>,
) -> mpsc::Receiver<E> {
    let (tx, rx) = mpsc::channel(64);
    let tx = Mutex::new(tx);
    notifier.register(move |event: &E| {
        executor::block_on(tx.lock().send(event.clone()));
    });
    rx
}
