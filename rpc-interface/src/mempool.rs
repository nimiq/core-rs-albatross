use async_trait::async_trait;
use nimiq_hash::Blake2bHash;
use nimiq_transaction::Transaction;

use crate::types::{HashOrTx, MempoolInfo, RPCResult};

#[nimiq_jsonrpc_derive::proxy(name = "MempoolProxy", rename_all = "camelCase")]
#[async_trait]
pub trait MempoolInterface {
    type Error;

    /// Pushes a raw transaction into the mempool, it will be assigned a default priority.
    async fn push_transaction(&mut self, raw_tx: String)
        -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Pushes a raw transaction into the mempool with high priority.
    async fn push_high_priority_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Obtains the list of transactions that are currently in the mempool.
    async fn mempool_content(
        &mut self,
        include_transactions: bool,
    ) -> RPCResult<Vec<HashOrTx>, (), Self::Error>;

    /// Obtains the mempool content in fee per byte buckets.
    async fn mempool(&mut self) -> RPCResult<MempoolInfo, (), Self::Error>;

    /// Obtains the minimum fee per byte as per mempool configuration.
    async fn get_min_fee_per_byte(&mut self) -> RPCResult<f64, (), Self::Error>;

    /// Tries to obtain the given transaction (using its hash) from the mempool.
    async fn get_transaction_from_mempool(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<Transaction, (), Self::Error>;
}
