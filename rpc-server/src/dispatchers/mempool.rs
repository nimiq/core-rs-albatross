use std::sync::Arc;

use async_trait::async_trait;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::{mempool::Mempool, mempool_transactions::TxPriority};
use nimiq_rpc_interface::{
    mempool::MempoolInterface,
    types::{HashOrTx, MempoolInfo, RPCResult},
};
use nimiq_serde::Deserialize;
use nimiq_transaction::Transaction;

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

    async fn push_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let tx: Transaction = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        let txid = tx.hash::<Blake2bHash>();

        match self.mempool.add_transaction(tx, None).await {
            Ok(_) => Ok(txid.into()),
            Err(e) => Err(Error::MempoolError(e)),
        }
    }

    async fn push_high_priority_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let tx: Transaction = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        let txid = tx.hash::<Blake2bHash>();

        match self
            .mempool
            .add_transaction(tx, Some(TxPriority::High))
            .await
        {
            Ok(_) => Ok(txid.into()),
            Err(e) => Err(Error::MempoolError(e)),
        }
    }

    async fn mempool_content(
        &mut self,
        include_transactions: bool,
    ) -> RPCResult<Vec<HashOrTx>, (), Self::Error> {
        return match include_transactions {
            true => Ok(self
                .mempool
                .get_transactions()
                .iter()
                .map(|tx| HashOrTx::from(tx.clone()))
                .collect::<Vec<_>>()
                .into()),
            false => Ok(self
                .mempool
                .get_transaction_hashes()
                .iter()
                .map(|hash| HashOrTx::from(hash.clone()))
                .collect::<Vec<_>>()
                .into()),
        };
    }

    async fn mempool(&mut self) -> RPCResult<MempoolInfo, (), Self::Error> {
        Ok(MempoolInfo::from_txs(self.mempool.get_transactions()).into())
    }

    async fn get_min_fee_per_byte(&mut self) -> RPCResult<f64, (), Self::Error> {
        Ok(self.mempool.get_rules().tx_fee_per_byte.into())
    }

    async fn get_transaction_from_mempool(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<Transaction, (), Self::Error> {
        if let Some(tx) = self.mempool.get_transaction_by_hash(&hash) {
            return Ok(tx.into());
        } else {
            return Err(Error::TransactionNotFound(hash));
        }
    }
}
