use std::sync::Arc;

use async_trait::async_trait;

use nimiq_network_libp2p::Network;

use nimiq_rpc_interface::{network::NetworkInterface, types::Peer};

use crate::error::Error;


pub struct NetworkDispatcher {
    network: Arc<Network>,
}

impl NetworkDispatcher {
    pub fn new(network: Arc<Network>) -> Self {
        NetworkDispatcher {
            network,
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl NetworkInterface for NetworkDispatcher {
    type Error = Error;

    async fn get_peer_id(&mut self) -> Result<String, Self::Error> {
        Ok(self.network.local_peer_id().to_string())
    }

    async fn get_peer_count(&mut self) -> Result<usize, Self::Error> {
        // TODO: Wait for connection pool implementation
        todo!()
    }

    async fn get_peer_list(&mut self) -> Result<Vec<Peer>, Self::Error> {
        // TODO: Wait for connection pool implementation
        todo!()
    }

    async fn get_peer_state(&mut self, peer_id: String) -> Result<Peer, Self::Error> {
        // TODO: Wait for connection pool implementation
        todo!()
    }
}
