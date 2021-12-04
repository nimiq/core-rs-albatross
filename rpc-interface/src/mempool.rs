use async_trait::async_trait;

use crate::types::{HashOrTx, MempoolInfo, Transaction};
use nimiq_hash::Blake2bHash;

#[nimiq_jsonrpc_derive::proxy(name = "MempoolProxy", rename_all = "camelCase")]
#[async_trait]
pub trait MempoolInterface {
    type Error;

    async fn push_transaction(&mut self, raw_tx: String) -> Result<Blake2bHash, Self::Error>;

    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
        check_mempool: Option<bool>,
    ) -> Result<Transaction, Self::Error>;

    async fn mempool_content(
        &mut self,
        include_transactions: bool,
    ) -> Result<Vec<HashOrTx>, Self::Error>;

    async fn mempool(&mut self) -> Result<MempoolInfo, Self::Error>;

    async fn get_min_fee_per_byte(&mut self) -> Result<f64, Self::Error>;
}
