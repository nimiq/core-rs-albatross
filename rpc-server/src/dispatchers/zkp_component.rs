use async_trait::async_trait;

use nimiq_network_libp2p::Network;
use nimiq_rpc_interface::types::ZKPState;
use nimiq_rpc_interface::zkp_component::ZKPComponentInterface;
use nimiq_zkp_component::zkp_component::ZKPComponentProxy;

use crate::error::Error;

pub struct ZKPComponentDispatcher {
    zkp_component: ZKPComponentProxy<Network>,
}

impl ZKPComponentDispatcher {
    pub fn new(zkp_component: ZKPComponentProxy<Network>) -> Self {
        ZKPComponentDispatcher { zkp_component }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl ZKPComponentInterface for ZKPComponentDispatcher {
    type Error = Error;

    /// Returns the peer ID for our local peer.
    async fn get_zkp_state(&mut self) -> Result<ZKPState, Self::Error> {
        Ok(ZKPState::with_zkp_state(
            &self.zkp_component.get_zkp_state(),
        ))
    }
}
