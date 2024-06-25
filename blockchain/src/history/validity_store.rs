use nimiq_database::{
    traits::{Database, ReadCursor, ReadTransaction, WriteTransaction},
    DatabaseProxy, TableFlags, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy::Policy;

/// The validity store is used by full nodes to keep track of which
/// transactions have occurred within the validity window without
/// having to store the full transactions
#[derive(Debug)]
pub struct ValidityStore {
    // Database handle.
    db: DatabaseProxy,
    // A database table with all the txn hashes
    pub(crate) txn_hashes: TableProxy,

    // A database table from block number to txn hashes
    pub(crate) block_txns: TableProxy,
}

impl ValidityStore {
    /// `RawTransactionHash` -> `u32` (block number)
    const TXN_HASHES_DB_NAME: &'static str = "ValidityTxnHashes";
    /// `u32` (block number) -> `RawTransactionHash`
    const BLOCK_TXNS_DB_NAME: &'static str = "ValidityBlockTxnHashes";

    /// Creates a new validity store initializing database tables
    pub(crate) fn new(db: DatabaseProxy) -> Self {
        let txn_hashes_table = db.open_table(Self::TXN_HASHES_DB_NAME.to_string());
        let block_txns_table = db.open_table_with_flags(
            Self::BLOCK_TXNS_DB_NAME.to_string(),
            TableFlags::DUPLICATE_KEYS | TableFlags::DUP_FIXED_SIZE_VALUES | TableFlags::UINT_KEYS,
        );

        Self {
            db,
            txn_hashes: txn_hashes_table,
            block_txns: block_txns_table,
        }
    }

    /// Returns true if the validity store has the given transaction hash.
    pub(crate) fn has_transaction(
        &self,
        txn_option: Option<&TransactionProxy>,
        raw_tx_hash: &Blake2bHash,
    ) -> bool {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Calculate first block in window.
        let validity_window_start = self
            .last_bn(txn)
            .saturating_sub(Policy::transaction_validity_window_blocks());

        // If the vector is empty then we have never seen a transaction with this hash.
        let block_number = txn.get::<Blake2bHash, u32>(&self.txn_hashes, raw_tx_hash);
        if let Some(block_number) = block_number {
            // If the transaction is inside the validity window, return true.
            if block_number > validity_window_start {
                return true;
            }
        }

        // If we didn't see any transaction inside the validity window then we can return false.
        false
    }

    /// Returns the first block number stored in the validity store
    pub(crate) fn first_bn(&self, db_tx: &TransactionProxy) -> u32 {
        // Initialize the cursor for the database.
        let mut cursor = db_tx.cursor(&self.block_txns);

        cursor.first::<u32, Blake2bHash>().unwrap_or_default().0
    }

    /// Obtains the last block number stored in the validity store.
    pub(crate) fn last_bn(&self, db_tx: &TransactionProxy) -> u32 {
        // Initialize the cursor for the database.
        let mut cursor = db_tx.cursor(&self.block_txns);

        cursor.last::<u32, Blake2bHash>().unwrap_or_default().0
    }

    /// Adds a transaction hash to the validity store
    pub(crate) fn add_transaction(
        &self,
        db_txn: &mut WriteTransactionProxy,
        block_number: u32,
        transaction: Blake2bHash,
    ) {
        db_txn.put(&self.txn_hashes, &transaction, &block_number);
        db_txn.put(&self.block_txns, &block_number, &transaction);
    }

    /// Delete the transactions associated to the given block number
    pub(crate) fn delete_block_transactions(
        &self,
        db_txn: &mut WriteTransactionProxy,
        block_number: u32,
    ) {
        if self.first_bn(db_txn) == self.last_bn(db_txn) {
            return;
        }

        log::trace!(bn = block_number, "Deleting block from validity store");

        let cursor = WriteTransaction::cursor(db_txn, &self.block_txns);

        let block_txns: Vec<Blake2bHash> = cursor
            .into_iter_dup_of::<_, Blake2bHash>(&block_number)
            .map(|(_, hash)| hash)
            .collect();

        for txn_hash in block_txns {
            db_txn.remove(&self.txn_hashes, &txn_hash);
        }

        db_txn.remove(&self.block_txns, &block_number);
    }

    /// Prunes the validity store keeping only 'validity_window_blocks'
    pub(crate) fn prune_validity_store(&self, db_txn: &mut WriteTransactionProxy) {
        // Compute the number of blocks we currently have in the store
        let first_bn = self.first_bn(db_txn);
        let last_bn = self.last_bn(db_txn);
        let num_blocks = last_bn - first_bn + 1;

        log::trace!(
            first = first_bn,
            latest = last_bn,
            blocks = num_blocks,
            "Pruning the validity store"
        );

        // We need to keep at least 'validity_window_blocks' + 'blocks_per_batch' blocks
        // to account for potential rebranches (when the window moves backwards).
        let num_blocks_to_keep =
            Policy::transaction_validity_window_blocks() + Policy::blocks_per_batch();

        if num_blocks > num_blocks_to_keep {
            // We need to prune the validity store
            let count = num_blocks - num_blocks_to_keep;

            for bn in first_bn..first_bn + count {
                self.delete_block_transactions(db_txn, bn);
            }
        }
    }

    /// Updates the validity store with the latest block number in the history store
    /// Note: Sometimes we have blocks that do not include transactions
    /// but we still need to track them in the validity store to mantain up to
    /// 'validity_window_blocks' inside the validity store
    pub(crate) fn update_validity_store(&self, db_txn: &mut WriteTransactionProxy, latest_bn: u32) {
        if latest_bn > self.last_bn(db_txn) {
            db_txn.put(&self.block_txns, &latest_bn, &Blake2bHash::default());
        }

        self.prune_validity_store(db_txn)
    }
}
