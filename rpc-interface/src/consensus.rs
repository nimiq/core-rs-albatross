use async_trait::async_trait;

use crate::{
    types::TransactionParameters,
};


#[cfg_attr(feature = "proxy", nimiq_jsonrpc_derive::proxy(name = "ConsensusProxy", rename_all="camelCase"))]
#[async_trait]
pub trait ConsensusInterface {
    type Error;

    async fn send_raw_transaction(&mut self, raw_tx: String) -> Result<String, Self::Error>;

    async fn create_raw_transaction(&mut self, tx_params: TransactionParameters) -> Result<String, Self::Error>;

    async fn send_transaction(&mut self, tx_params: TransactionParameters) -> Result<String, Self::Error>;
}
