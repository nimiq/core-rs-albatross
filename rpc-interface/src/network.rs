use async_trait::async_trait;

use crate::types::Peer;


#[cfg_attr(
    feature = "proxy",
    nimiq_jsonrpc_derive::proxy(name = "NetworkProxy", rename_all = "camelCase")
)]
#[async_trait]
pub trait NetworkInterface {
    type Error;

    async fn get_peer_id(&mut self) -> Result<String, Self::Error>;

    async fn get_peer_count(&mut self) -> Result<usize, Self::Error>;

    async fn get_peer_list(&mut self) -> Result<Vec<Peer>, Self::Error>;

    async fn get_peer_state(&mut self, peer_id: String) -> Result<Peer, Self::Error>;
}
