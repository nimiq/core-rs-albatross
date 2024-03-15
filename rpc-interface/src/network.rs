use async_trait::async_trait;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "NetworkProxy", rename_all = "camelCase")]
#[async_trait]
pub trait NetworkInterface {
    type Error;

    /// Returns the peer ID for our local peer.
    async fn get_peer_id(&mut self) -> RPCResult<String, (), Self::Error>;

    /// Returns the number of peers.
    async fn get_peer_count(&mut self) -> RPCResult<usize, (), Self::Error>;

    /// Returns a list with the IDs of all our peers.
    async fn get_peer_list(&mut self) -> RPCResult<Vec<String>, (), Self::Error>;
}
