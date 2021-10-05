use async_trait::async_trait;

#[cfg_attr(
    feature = "proxy",
    nimiq_jsonrpc_derive::proxy(name = "NetworkProxy", rename_all = "camelCase")
)]
#[async_trait]
pub trait NetworkInterface {
    type Error;

    async fn get_peer_id(&mut self) -> Result<String, Self::Error>;

    async fn get_peer_count(&mut self) -> Result<usize, Self::Error>;

    async fn get_peer_list(&mut self) -> Result<Vec<String>, Self::Error>;
}
