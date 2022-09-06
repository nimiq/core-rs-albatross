use crate::types::RPCResult;
use async_trait::async_trait;

#[nimiq_jsonrpc_derive::proxy(name = "NetworkProxy", rename_all = "camelCase")]
#[async_trait]
pub trait NetworkInterface {
    type Error;

    async fn get_peer_id(&mut self) -> RPCResult<String, (), Self::Error>;

    async fn get_peer_count(&mut self) -> RPCResult<usize, (), Self::Error>;

    async fn get_peer_list(&mut self) -> RPCResult<Vec<String>, (), Self::Error>;
}
