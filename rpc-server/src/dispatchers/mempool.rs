use std::sync::Arc;

use async_trait::async_trait;
use beserial::Deserialize;

use nimiq_blockchain::AbstractBlockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::mempool::Mempool;

use nimiq_rpc_interface::mempool::MempoolInterface;
use nimiq_rpc_interface::types::{HashOrTx, MempoolInfo, Transaction};

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

        match self.mempool.add_transaction(tx).await {
            Ok(_) => Ok(txid),
            Err(e) => Err(Error::MempoolError(e)),
        }
    }

    /// Tries to fetch a transaction (including reward transactions) given its hash. It has an option
    /// to also search the mempool for the transaction, it defaults to false.
    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
        check_mempool: Option<bool>,
    ) -> Result<Transaction, Error> {
        // First check the mempool.
        if check_mempool == Some(true) {
            if let Some(tx) = self.mempool.get_transaction_by_hash(&hash) {
                return Ok(Transaction::from_transaction(tx));
            }
        }

        // Now check the blockchain.
        let blockchain = self.mempool.blockchain.read();

        // Get all the extended transactions that correspond to this hash.
        let mut extended_tx_vec = blockchain.history_store.get_ext_tx_by_hash(&hash, None);

        // Unpack the transaction or raise an error.
        let extended_tx = match extended_tx_vec.len() {
            0 => {
                return Err(Error::TransactionNotFound(hash));
            }
            1 => extended_tx_vec.pop().unwrap(),
            _ => {
                return Err(Error::MultipleTransactionsFound(hash));
            }
        };

        // Convert the extended transaction into a regular transaction. This will also convert
        // reward inherents.
        let block_number = extended_tx.block_number;
        let timestamp = extended_tx.block_time;

        return match extended_tx.into_transaction() {
            Ok(tx) => Ok(Transaction::from_blockchain(
                tx,
                block_number,
                timestamp,
                blockchain.block_number(),
            )),
            Err(_) => Err(Error::TransactionNotFound(hash)),
        };
    }

    async fn mempool_content(
        &mut self,
        include_transactions: bool,
    ) -> Result<Vec<HashOrTx>, Error> {
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

    async fn mempool(&mut self) -> Result<MempoolInfo, Error> {
        Ok(MempoolInfo::from_txs(self.mempool.get_transactions()))
    }

    async fn get_min_fee_per_byte(&mut self) -> Result<f64, Self::Error> {
        Ok(self.mempool.get_rules().tx_fee_per_byte)
    }
}
