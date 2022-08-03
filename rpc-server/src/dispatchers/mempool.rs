use std::sync::Arc;

use async_trait::async_trait;
use beserial::Deserialize;

use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::mempool::Mempool;

use nimiq_mempool::mempool_transactions::TxPriority;
use nimiq_rpc_interface::mempool::MempoolInterface;
use nimiq_rpc_interface::types::{HashOrTx, MempoolInfo};

use crate::error::Error;

#[allow(dead_code)]
pub struct MempoolDispatcher {
    mempool: Arc<Mempool>,
}

impl MempoolDispatcher {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        MempoolDispatcher { mempool }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl MempoolInterface for MempoolDispatcher {
    type Error = Error;

    /// Pushes the given serialized transaction to the local mempool.
    async fn push_transaction(&mut self, raw_tx: String) -> Result<Blake2bHash, Self::Error> {
        let tx: nimiq_transaction::Transaction =
            Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        let txid = tx.hash::<Blake2bHash>();

        match self.mempool.add_transaction(tx, None).await {
            Ok(_) => Ok(txid),
            Err(e) => Err(Error::MempoolError(e)),
        }
    }

    /// Pushes the given serialized transaction to the local mempool with high priority
    async fn push_high_priority_transaction(
        &mut self,
        raw_tx: String,
    ) -> Result<Blake2bHash, Self::Error> {
        let tx: nimiq_transaction::Transaction =
            Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        let txid = tx.hash::<Blake2bHash>();

        match self
            .mempool
            .add_transaction(tx, Some(TxPriority::HighPriority))
            .await
        {
            Ok(_) => Ok(txid),
            Err(e) => Err(Error::MempoolError(e)),
        }
    }

    async fn mempool_content(
        &mut self,
        include_transactions: bool,
    ) -> Result<Vec<HashOrTx>, Self::Error> {
        return match include_transactions {
            true => Ok(self
                .mempool
                .get_transactions()
                .iter()
                .map(|tx| HashOrTx::from(tx.clone()))
                .collect()),
            false => Ok(self
                .mempool
                .get_transaction_hashes()
                .iter()
                .map(|hash| HashOrTx::from(hash.clone()))
                .collect()),
        };
    }

    async fn mempool(&mut self) -> Result<MempoolInfo, Self::Error> {
        Ok(MempoolInfo::from_txs(self.mempool.get_transactions()))
    }

    async fn get_min_fee_per_byte(&mut self) -> Result<f64, Self::Error> {
        Ok(self.mempool.get_rules().tx_fee_per_byte)
    }
}
