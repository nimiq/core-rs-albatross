use std::sync::Arc;

use async_trait::async_trait;
use nimiq_network_interface::{
    network::Network as InterfaceNetwork,
    peer_info::{NodeType, Services},
};
use nimiq_network_libp2p::Network;
use nimiq_rpc_interface::{
    network::NetworkInterface,
    types::{PeerType, RPCResult},
};
use nimiq_utils::CARGO_VERSION;

use crate::error::Error;

pub struct NetworkDispatcher {
    network: Arc<Network>,
}

impl NetworkDispatcher {
    pub fn new(network: Arc<Network>) -> Self {
        NetworkDispatcher { network }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl NetworkInterface for NetworkDispatcher {
    type Error = Error;

    async fn get_peer_id(&self) -> RPCResult<String, (), Self::Error> {
        Ok(self.network.local_peer_id().to_string().into())
    }

    async fn get_peer_count(&self) -> RPCResult<usize, (), Self::Error> {
        Ok(self.network.get_peers().len().into())
    }

    async fn get_peer_list(&self) -> RPCResult<Vec<String>, (), Self::Error> {
        Ok(self
            .network
            .get_peers()
            .into_iter()
            .map(|peer_id| peer_id.to_string())
            .collect::<Vec<_>>()
            .into())
    }

    async fn get_address_book(&self) -> RPCResult<Vec<(String, PeerType)>, (), Self::Error> {
        let address_book = self.network.get_address_book();

        let mut rpc_address_book = Vec::new();

        for (peer_id, peer_info) in address_book {
            let peer_type = if peer_info
                .get_services()
                .contains(Services::provided(NodeType::History))
            {
                PeerType::History
            } else if peer_info
                .get_services()
                .contains(Services::provided(NodeType::Full))
            {
                PeerType::Full
            } else {
                PeerType::Light
            };

            rpc_address_book.push((peer_id.to_string(), peer_type));
        }

        Ok(rpc_address_book.into())
    }

    async fn get_client_version(&self) -> RPCResult<String, (), Self::Error> {
        Ok(String::from(CARGO_VERSION).into())
    }
}
