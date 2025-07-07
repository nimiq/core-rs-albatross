use async_trait::async_trait;

use crate::types::{PeerType, RPCResult};

#[nimiq_jsonrpc_derive::proxy(name = "NetworkProxy", rename_all = "camelCase")]
#[async_trait]
pub trait NetworkInterface {
    type Error;

    /// Returns the peer ID for our local peer.
    async fn get_peer_id(&self) -> RPCResult<String, (), Self::Error>;

    /// Returns the number of peers.
    async fn get_peer_count(&self) -> RPCResult<usize, (), Self::Error>;

    /// Returns a list with the IDs of all our peers.
    async fn get_peer_list(&self) -> RPCResult<Vec<String>, (), Self::Error>;

    /// Returns the address book
    async fn get_address_book(&self) -> RPCResult<Vec<(String, PeerType)>, (), Self::Error>;

    /// Returns the version of the client the RPC server is running.
    async fn get_client_version(&self) -> RPCResult<String, (), Self::Error>;
}
