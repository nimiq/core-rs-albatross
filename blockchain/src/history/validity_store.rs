use nimiq_database::{
    declare_table,
    mdbx::{MdbxDatabase, MdbxReadTransaction, MdbxWriteTransaction, OptionalTransaction},
    traits::{Database, DupReadCursor, ReadCursor, ReadTransaction, WriteTransaction},
};
use nimiq_primitives::policy::Policy;
use nimiq_transaction::historic_transaction::RawTransactionHash;

// `RawTransactionHash` -> `u32` (block number)
declare_table!(TxnHashesTable, "ValidityTxnHashes", RawTransactionHash => u32);
// `u32` (block number) -> `RawTransactionHash`
declare_table!(BlockTxnsTable, "ValidityBlockTxnHashes", u32 => dup(RawTransactionHash));

/// The validity store is used by full/history nodes to keep track of which
/// transactions have occurred within the validity window.
/// The validity store keeps at least `validity_window_blocks + blocks_per_batch` blocks
#[derive(Debug)]
pub struct ValidityStore {
    // Database handle.
    db: MdbxDatabase,
    // A database table with all the txn hashes
    pub(crate) txn_hashes: TxnHashesTable,

    // A database table from block number to txn hashes
    pub(crate) block_txns: BlockTxnsTable,
}

impl ValidityStore {
    /// Creates a new validity store initializing database tables
    pub(crate) fn new(db: MdbxDatabase) -> Self {
        let store = Self {
            db,
            txn_hashes: TxnHashesTable,
            block_txns: BlockTxnsTable,
        };

        store.db.create_regular_table(&store.txn_hashes);
        store.db.create_dup_table(&store.block_txns);

        store
    }

    /// Returns true if the validity store has the given transaction hash.
    pub(crate) fn has_transaction(
        &self,
        txn_option: Option<&MdbxReadTransaction>,
        raw_tx_hash: &RawTransactionHash,
    ) -> bool {
        let txn = txn_option.or_new(&self.db);

        // Calculate first block in window.
        let validity_window_start = self
            .last_bn(&txn)
            .saturating_sub(Policy::transaction_validity_window_blocks());

        // If the vector is empty then we have never seen a transaction with this hash.
        let block_number = txn.get(&self.txn_hashes, raw_tx_hash);
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
    pub(crate) fn first_bn(&self, db_tx: &MdbxReadTransaction) -> u32 {
        // Initialize the cursor for the database.
        let mut cursor = db_tx.dup_cursor(&self.block_txns);

        cursor.first().map(|(k, _v)| k).unwrap_or_default()
    }

    /// Obtains the last block number stored in the validity store.
    pub(crate) fn last_bn(&self, db_tx: &MdbxReadTransaction) -> u32 {
        // Initialize the cursor for the database.
        let mut cursor = db_tx.dup_cursor(&self.block_txns);

        cursor.last().map(|(k, _v)| k).unwrap_or_default()
    }

    /// Adds a transaction hash to the validity store
    pub(crate) fn add_transaction(
        &self,
        db_txn: &mut MdbxWriteTransaction,
        block_number: u32,
        transaction_hash: RawTransactionHash,
    ) {
        db_txn.put(&self.txn_hashes, &transaction_hash, &block_number);
        db_txn.put(&self.block_txns, &block_number, &transaction_hash);
    }

    /// Delete the transactions associated to the given block number
    pub(crate) fn delete_block_transactions(
        &self,
        db_txn: &mut MdbxWriteTransaction,
        block_number: u32,
    ) {
        if self.first_bn(db_txn) == self.last_bn(db_txn) {
            return;
        }

        log::trace!(bn = block_number, "Deleting block from validity store");

        let cursor = WriteTransaction::dup_cursor(db_txn, &self.block_txns);

        let block_txns: Vec<RawTransactionHash> = cursor
            .into_iter_dup_of(&block_number)
            .map(|(_, hash)| hash)
            .collect();

        for txn_hash in block_txns {
            db_txn.remove(&self.txn_hashes, &txn_hash);
        }

        db_txn.remove(&self.block_txns, &block_number);
    }

    /// Prunes the validity store keeping only 'validity_window_blocks'
    pub(crate) fn prune_validity_store(&self, db_txn: &mut MdbxWriteTransaction) {
        // Compute the number of blocks we currently have in the store
        let first_bn = self.first_bn(db_txn);
        let last_bn = self.last_bn(db_txn);
        assert!(
            first_bn <= last_bn,
            "First block number {} is greater than last block number {}",
            first_bn,
            last_bn
        );
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
    pub(crate) fn update_validity_store(&self, db_txn: &mut MdbxWriteTransaction, latest_bn: u32) {
        if latest_bn > self.last_bn(db_txn) {
            db_txn.put(&self.block_txns, &latest_bn, &RawTransactionHash::default());
        }

        self.prune_validity_store(db_txn)
    }
}
