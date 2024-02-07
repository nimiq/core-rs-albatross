use std::collections::{HashMap, HashSet};

use nimiq_block::Block;
use nimiq_database::{
    traits::{Database, ReadCursor, ReadTransaction, WriteCursor, WriteTransaction},
    DatabaseProxy, TableFlags, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::{Blake2bHash, Blake2bHasher, Hash, Hasher};
use nimiq_primitives::policy::Policy;
use nimiq_transaction::historic_transaction::{HistoricTransaction, RawTransactionHash};

/// The validity store is used by full nodes to keep track of which
/// transactions have occurred within the validity window without
/// having to store the full transactions
pub struct ValidityStore {
    /// The set of all raw transactions of the current validity window.
    pub(crate) txs: HashSet<Blake2bHash>,

    pub(crate) block_txs: HashMap<u32, Vec<Blake2bHash>>,
    /// Database handle.
    db: DatabaseProxy,
    /// A database of all transaction hashes prefixed by the batch number.
    pub(crate) txs_table: TableProxy,
}

impl ValidityStore {
    const VALIDITY_STORE_DB_NAME: &'static str = "ValidityStoreTransactions";

    pub(crate) fn new(db: DatabaseProxy) -> Self {
        let txs_table = db.open_table(Self::VALIDITY_STORE_DB_NAME.to_string());

        Self {
            txs: HashSet::new(),
            block_txs: HashMap::new(),
            db,
            txs_table,
        }
    }

    pub(crate) fn has_transaction(&self, raw_tx_hash: RawTransactionHash) -> bool {
        self.txs.contains(&raw_tx_hash.into())
    }

    pub(crate) fn add_block_transactions(
        &mut self,
        txn: &mut WriteTransactionProxy,
        block: &Block,
    ) {
        let batch_number = Policy::batch_at(block.block_number());

        if let Some(txs) = block.transactions() {
            for tx in txs {
                let raw_tx_hash = tx.raw_tx_hash();
                let _leaf_hash = Self::prefixed_hash_tx(batch_number, &raw_tx_hash);

                self.block_txs
                    .get_mut(block.block_number())
                    .unwrap_or(self.block_txs.inse)
                    .insert(block.block_number(), raw_tx_hash.clone().into());
                self.txs.insert(raw_tx_hash.into());
            }
        }
    }

    pub(crate) fn delete_block_transactions(
        &mut self,
        txn: &mut WriteTransactionProxy,
        block_number: u32,
    ) -> Option<()> {
        let txs = self.block_txs.remove(&block_number)?;

        txs.Some(())
    }

    /// Returns the hash for the batch number and the raw transaction hash.
    fn prefixed_hash_tx(batch_number: u32, raw_tx_hash: &RawTransactionHash) -> Blake2bHash {
        let mut message = batch_number.to_be_bytes().to_vec();
        message.append(&mut raw_tx_hash.0.to_vec());

        message.hash()
    }
}
