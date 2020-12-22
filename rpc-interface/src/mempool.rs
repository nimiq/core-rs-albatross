use async_trait::async_trait;

use nimiq_hash::Blake2bHash;


#[cfg_attr(feature = "proxy", nimiq_jsonrpc_derive::proxy(name = "MempoolProxy"))]
#[async_trait]
pub trait MempoolInterface {
    type Error;

    async fn get_transaction(&mut self, txid: Blake2bHash) -> Result<Option<()>, Self::Error>;

    async fn mempool_content(&mut self, include_transactions: bool) -> Result<Vec<()>, Self::Error>;

    async fn mempool(&mut self) -> Result<(), Self::Error>;

    async fn get_mempool_transaction(&mut self) -> Result<(), Self::Error>;
}
