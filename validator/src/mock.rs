use std::sync::Arc;

use failure::_core::pin::Pin;
use futures::task::{Context, Poll};
use futures::{executor, Future, Stream};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use block_albatross::{MacroBlock, SignedViewChange, ViewChangeProof};
use blockchain_albatross::Blockchain;
use network::Network;
use network_interface::network::Network as NetworkInterface;
use nimiq_network_mock::network::MockNetwork;
use primitives::slot::ValidatorSlots;
use utils::observer::Notifier;

pub trait ValidatorNetwork: NetworkInterface {}
impl ValidatorNetwork for Network {}
impl ValidatorNetwork for MockNetwork {}

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
        signed_view_change: SignedViewChange,
        validator_id: u16,
        active_validators: ValidatorSlots,
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
