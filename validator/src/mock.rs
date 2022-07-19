use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use nimiq_block::{MacroBlock, SignedViewChange, ViewChangeProof};
use nimiq_network::Network;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_mock::MockNetwork;
use nimiq_primitives::slot::ValidatorSlots;

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

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }
}

pub struct ViewChangeHandel;
impl ViewChangeHandel {
    pub fn new(
        _signed_view_change: SignedViewChange,
        _validator_id: u16,
        _active_validators: ValidatorSlots,
    ) -> Self {
        unimplemented!()
    }
}
impl Future for ViewChangeHandel {
    type Output = ViewChangeProof;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }
}
