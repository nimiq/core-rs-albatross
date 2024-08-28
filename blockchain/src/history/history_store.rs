use std::{cmp, ops::Range};

use nimiq_block::MicroBlock;
use nimiq_database::{
    declare_table,
    mdbx::{MdbxDatabase, MdbxReadTransaction, MdbxWriteTransaction, OptionalTransaction},
    traits::{
        Database, DupReadCursor, DupWriteCursor, ReadCursor, ReadTransaction, WriteCursor,
        WriteTransaction,
    },
};
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mmr::{
    error::Error as MMRError,
    mmr::{
        partial::PartialMerkleMountainRange,
        position::leaf_number_to_index,
        proof::{RangeProof, SizeProof},
        MerkleMountainRange,
    },
    store::memory::MemoryStore,
};
use nimiq_primitives::policy::Policy;
use nimiq_serde::Serialize;
use nimiq_transaction::{
    historic_transaction::{
        EquivocationEvent, HistoricTransaction, HistoricTransactionData, RawTransactionHash,
    },
    history_proof::HistoryTreeProof,
    inherent::Inherent,
    EquivocationLocator,
};

use super::{
    interface::HistoryInterface, utils::IndexedTransaction, validity_store::ValidityStore,
};
use crate::history::{mmr_store::MMRStore, HistoryTreeChunk};

// `u32` (epoch number) -> `IndexedHash` (`node_index || Blake2bHash`)
declare_table!(HistoryTreeTable, "HistoryTrees", u32 => u32 => Blake2bHash);
// `u32` (epoch number) -> `IndexedTransaction` (`leaf_index || HistoricTransaction`)
declare_table!(HistoricTransactionTable, "HistoricTransactions", u32 => u32 => HistoricTransaction);
declare_table!(LastLeafTable, "LastLeafIndexesByBlock", u32 => u32);

/// A struct that contains databases to store history trees (which are Merkle Mountain Ranges
/// constructed from the list of historic transactions in an epoch) and historic transactions (which
/// are representations of transactions).
/// The history trees allow a node in possession of a transaction to prove to another node (that
/// only has macro block headers) that that given transaction happened.
pub struct HistoryStore {
    /// Database handle.
    db: MdbxDatabase,
    /// A database of all history trees indexed by their epoch number.
    hist_tree_table: HistoryTreeTable,
    /// A database of all historic transactions indexed by their epoch number and leaf index.
    pub(super) hist_tx_table: HistoricTransactionTable,
    /// A database of the last leaf index for each block number.
    last_leaf_table: LastLeafTable,

    /// The validity store is used by nodes to keep track of which
    /// transactions have occurred within the validity window.
    pub(crate) validity_store: ValidityStore,

    /// The network ID. It determines if this is the mainnet or one of the testnets.
    pub(crate) network_id: NetworkId,
}

impl HistoryStore {
    /// Creates a new HistoryStore.
    pub fn new(db: MdbxDatabase, network_id: NetworkId) -> Self {
        let store = HistoryStore {
            validity_store: ValidityStore::new(db.clone()),
            db,
            network_id,
            hist_tree_table: HistoryTreeTable,
            hist_tx_table: HistoricTransactionTable,
            last_leaf_table: LastLeafTable,
        };

        store.db.create_dup_table(&store.hist_tree_table);
        store.db.create_dup_table(&store.hist_tx_table);
        store.db.create_regular_table(&store.last_leaf_table);

        store
    }

    /// Gets an historic transaction by its hash. Note that this hash is the leaf hash (see MMRHash)
    /// of the transaction, not a simple Blake2b hash of the transaction.
    pub(crate) fn get_historic_tx(
        &self,
        epoch_number: u32,
        leaf_index: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoricTransaction> {
        let txn = txn_option.or_new(&self.db);

        let mut cursor = txn.dup_cursor(&self.hist_tx_table);
        let value = cursor.set_subkey(&epoch_number, &leaf_index)?;
        Some(value.value)
    }

    fn get_historic_txns(
        &self,
        epoch_number: u32,
        leaf_indices: Range<u32>,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        let mut hist_txs = Vec::with_capacity(leaf_indices.len());
        let txn = txn_option.or_new(&self.db);

        // Get consecutive transactions with fast cursor.
        let mut cursor = txn.dup_cursor(&self.hist_tx_table);

        for (i, leaf_index) in leaf_indices.enumerate() {
            let (epoch, hist_tx) = if i == 0 {
                (
                    epoch_number,
                    cursor
                        .set_subkey(&epoch_number, &leaf_index)
                        .expect("Transaction not found"),
                )
            } else {
                cursor.next_duplicate().expect("Transaction not found")
            };

            assert_eq!(epoch, epoch_number, "Epoch number mismatch");
            assert_eq!(hist_tx.index, leaf_index, "Index mismatch");
            hist_txs.push(hist_tx.value);
        }

        hist_txs
    }

    /// Internal method to remove leaves from the history tree.
    /// Returns the root and leaf indices.
    pub(crate) fn remove_leaves_from_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        limit: Option<usize>,
    ) -> Option<(Blake2bHash, Range<u32>)> {
        // Get the history tree.
        let mut tree = MerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_table,
            txn,
            epoch_number,
        ));

        // Get the history root. We need to get it here because of Rust's borrowing rules.
        let root = tree.get_root().ok()?;

        // Remove all leaves from the history tree and remember the respective hashes and indexes.
        let num_leaves = tree.num_leaves();

        // Optimisation if we remove all leaves.
        if limit.is_none() || limit == Some(num_leaves) {
            txn.remove(&self.hist_tree_table, &epoch_number);
            return Some((root, 0..num_leaves as u32));
        }

        let num_hist_txs = limit.map(|l| cmp::min(l, num_leaves)).unwrap_or(num_leaves);
        let lower_limit = (num_leaves - num_hist_txs) as u32;

        for _ in 0..num_hist_txs {
            tree.remove_back().ok()?;
        }
        Some((root, lower_limit..num_leaves as u32))
    }

    pub(crate) fn remove_txns_from_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        leaf_indices: Range<u32>,
    ) -> u64 {
        let mut txns_size = 0u64;

        let mut cursor = WriteTransaction::dup_cursor(txn, &self.hist_tx_table);

        for (i, leaf_index) in leaf_indices.rev().enumerate() {
            let tx_opt: Option<IndexedTransaction> = if i == 0 {
                cursor.set_subkey(&epoch_number, &leaf_index)
            } else {
                // The entries should be consecutive in the database,
                // so we can just call next() without seeking.
                cursor.prev_duplicate().map(|(k, v)| {
                    assert_eq!(epoch_number, k, "Invalid order of leaf indices");
                    v
                })
            };

            let Some(hist_tx) = tx_opt else { continue };
            assert_eq!(hist_tx.index, leaf_index, "Invalid leaf index");
            let hist_tx = hist_tx.value;

            // Remove it from the historic transaction database.
            cursor.remove();

            txns_size += hist_tx.serialized_size() as u64;

            // Remove it from the leaf index database.
            // Check if you are removing the last historic transaction for this block. If yes,
            // completely remove the block, if not just decrement the last leaf index.
            let block_number = hist_tx.block_number;
            let (start, end) = self.get_indexes_for_block(block_number, Some(txn));

            if end - start == 1 {
                txn.remove(&self.last_leaf_table, &block_number);
            } else {
                txn.put(&self.last_leaf_table, &block_number, &(leaf_index - 1));
            }
        }

        txns_size
    }

    pub(crate) fn remove_epoch_from_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
    ) {
        // Fast removal of all transactions.
        txn.remove(&self.hist_tx_table, &epoch_number);

        let mut cursor = WriteTransaction::cursor(txn, &self.last_leaf_table);

        let Some(first_block) = Policy::first_block_of(epoch_number) else {
            return;
        };

        let Some((mut block_number, _value)) = cursor.set_lowerbound_key(&first_block) else {
            return;
        };

        while Policy::epoch_at(block_number) == epoch_number {
            cursor.remove();
            let Some((block, _value)) = cursor.next() else {
                return;
            };
            block_number = block;
        }
    }

    /// Returns a proof for all the historic transactions at the given positions (leaf indexes). The
    /// proof also includes the historic transactions.
    pub(crate) fn prove_with_position(
        &self,
        epoch_number: u32,
        leaf_indices: Vec<usize>,
        verifier_state: Option<usize>,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoryTreeProof> {
        let txn = txn_option.or_new(&self.db);

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            epoch_number,
        ));

        // Create Merkle proof.
        let proof = tree.prove(&leaf_indices, verifier_state).ok()?;

        // Get each historic transaction from the tree.
        let mut hist_txs = vec![];

        for i in &leaf_indices {
            hist_txs.push(
                self.get_historic_tx(epoch_number, *i as u32, Some(&txn))
                    .unwrap(),
            );
        }

        Some(HistoryTreeProof {
            proof,
            positions: leaf_indices,
            history: hist_txs,
        })
    }

    /// Returns the range of leaf indexes corresponding to the given block number.
    pub(crate) fn get_indexes_for_block(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> (u32, u32) {
        let txn = txn_option.or_new(&self.db);

        // Seek to the last leaf index of the block, if it exists.
        let mut cursor = txn.cursor(&self.last_leaf_table);

        let end = match cursor.set_key(&block_number) {
            // If the block number doesn't exist in the database that's because it doesn't contain
            // any transactions or inherents. So we terminate here.
            None => return (0, 0),
            // Otherwise, we simply get the last leaf index for the block. We increment by 1 because
            // we want the range that we return to be non-inclusive, i.e. [a, b).
            Some(i) => i + 1,
        };

        let start = if Policy::epoch_index_at(block_number) == 0 {
            // If this is the first block of the epoch then it starts at zero by definition.
            0
        } else {
            // Otherwise, seek to the last leaf index of the previous block in the database.
            match cursor.prev() {
                // If it doesn't exist, then we have to start at zero.
                None => 0,
                Some((n, i)) => {
                    // If the previous block is from a different epoch, then we also have to start at zero.
                    if Policy::epoch_at(n) != Policy::epoch_at(block_number) {
                        0
                    } else {
                        i + 1
                    }
                }
            }
        };

        (start, end)
    }

    /// Calculates the history tree root from a vector of historic transactions. It doesn't use the
    /// database, it is just used to check the correctness of the history root when syncing.
    pub(crate) fn _root_from_hist_txs(hist_txs: &[HistoricTransaction]) -> Option<Blake2bHash> {
        // Create a new history tree.
        let mut tree = MerkleMountainRange::new(MemoryStore::new());

        // Append the historic transactions to the history tree.
        for tx in hist_txs {
            tree.push(tx).ok()?;
        }

        // Return the history root.
        tree.get_root().ok()
    }

    /// Internal function for `add_to_history`, which also returns leaf indices.
    pub(crate) fn put_historic_txns(
        &self,
        txn: &mut MdbxWriteTransaction,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64, Vec<u32>)> {
        let epoch_number = Policy::epoch_at(block_number);

        // Get the history tree.
        let mut tree = MerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_table,
            txn,
            epoch_number,
        ));

        // Append the historic transactions to the history tree and keep the respective leaf indexes.
        let mut leaf_idx = vec![];

        for tx in hist_txs {
            let i = tree.push(tx).ok()?;
            leaf_idx.push(i as u32);
        }

        let root = tree.get_root().ok()?;
        let mut txns_size = 0u64;

        // Add the historic transactions into the respective database.
        // We need to do this separately due to the borrowing rules of Rust.
        let mut cursor = WriteTransaction::dup_cursor(txn, &self.hist_tx_table);
        for (hist_tx, &leaf_index) in hist_txs.iter().zip(leaf_idx.iter()) {
            assert!(
                hist_tx.block_number <= block_number
                    && Policy::epoch_at(hist_tx.block_number) == Policy::epoch_at(block_number),
                "Inconsistent transactions when adding to history store (block #{}, tx block #{}).",
                block_number,
                hist_tx.block_number
            );

            let value = IndexedTransaction {
                index: leaf_index,
                value: hist_tx.clone(),
            };
            cursor.append_dup(&epoch_number, &value);

            self.validity_store
                .add_transaction(txn, hist_tx.block_number, hist_tx.tx_hash());

            txn.put(&self.last_leaf_table, &hist_tx.block_number, &leaf_index);

            txns_size += hist_tx.serialized_size() as u64;
        }

        self.validity_store.update_validity_store(txn, block_number);

        // Return the history root.
        Some((root, txns_size, leaf_idx))
    }
}

impl HistoryInterface for HistoryStore {
    fn clear(&self, txn: &mut MdbxWriteTransaction) {
        txn.clear_table(&self.hist_tree_table);
        txn.clear_table(&self.hist_tx_table);
        txn.clear_table(&self.last_leaf_table);
    }

    /// Returns the length (i.e. the number of leaves) of the History Tree at a given block height.
    /// Note that this returns the number of leaves for only the epoch of the given block height,
    /// this is because we have separate History Trees for separate epochs.
    fn length_at(&self, block_number: u32, txn_option: Option<&MdbxReadTransaction>) -> u32 {
        let txn = txn_option.or_new(&self.db);

        let mut cursor = txn.cursor(&self.last_leaf_table);

        // Seek to the last leaf index of the block, if it exists.
        match cursor.set_lowerbound_key(&block_number) {
            // If it exists, we simply get the last leaf index for the block. We increment by 1
            // because the leaf index is 0-based and we want the number of leaves.
            Some((n, i)) if n == block_number => i + 1,
            // Otherwise, seek to the previous block, if it exists.
            _ => match cursor.prev() {
                // If it exists, we also need to check if the previous block is in the same epoch.
                Some((n, i)) => {
                    if Policy::epoch_at(n) == Policy::epoch_at(block_number) {
                        i + 1
                    } else {
                        0
                    }
                }
                // If it doesn't exist, then the HistoryStore is empty at this block height.
                None => 0,
            },
        }
    }

    /// Returns the total length of the History Tree at a given epoch number.
    /// The size of the history length is useful for getting a proof for a previous state
    /// of the history tree.
    fn total_len_at_epoch(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        let txn = txn_option.or_new(&self.db);
        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            epoch_number,
        ));

        // Get the Merkle tree length
        tree.len()
    }

    /// Add a list of historic transactions to an existing history tree. It returns the root of the
    /// resulting tree and the total size of the transactions added.
    /// This function assumes that:
    ///     1. The transactions are pushed in increasing block number order.
    ///     2. All the blocks are consecutive.
    ///     3. We only push transactions for one epoch at a time.
    /// This method will fail if we try to push transactions from previous epochs.
    fn add_to_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        self.put_historic_txns(txn, block_number, hist_txs)
            .map(|(root, size, _)| (root, size))
    }

    /// Removes a number of historic transactions from an existing history tree. It returns the root
    /// of the resulting tree and the total size of the transactions removed.
    fn remove_partial_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        num_hist_txs: usize,
    ) -> Option<(Blake2bHash, u64)> {
        let (root, leaf_indices) =
            self.remove_leaves_from_history(txn, epoch_number, Some(num_hist_txs))?;

        // Remove each of the historic transactions in the history tree from the extended
        // transaction database.
        let txns_size = self.remove_txns_from_history(txn, epoch_number, leaf_indices);

        // Return the history root.
        Some((root, txns_size))
    }

    /// Removes an existing history tree and all the historic transactions that were part of it.
    /// Returns None if there's no history tree corresponding to the given epoch number.
    fn remove_history(&self, txn: &mut MdbxWriteTransaction, epoch_number: u32) -> Option<()> {
        self.remove_leaves_from_history(txn, epoch_number, None)?;
        self.remove_epoch_from_history(txn, epoch_number);

        Some(())
    }

    /// Gets the history tree root for a given epoch.
    fn get_history_tree_root(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<Blake2bHash> {
        let txn = txn_option.or_new(&self.db);

        // Get the history tree.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            Policy::epoch_at(block_number),
        ));

        // Return the history root.
        tree.get_root().ok()
    }

    fn tx_in_validity_window(
        &self,
        raw_tx_hash: &RawTransactionHash,
        txn_opt: Option<&MdbxReadTransaction>,
    ) -> bool {
        self.validity_store.has_transaction(txn_opt, raw_tx_hash)
    }

    /// Gets all historic transactions for a given block number.
    /// This method returns the transactions in the same order that they appear in the block.
    fn get_block_transactions(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        let txn = txn_option.or_new(&self.db);

        // Get the history tree.
        let epoch_number = Policy::epoch_at(block_number);

        // Get the range of leaf indexes at this height.
        let (start, end) = self.get_indexes_for_block(block_number, Some(&txn));

        self.get_historic_txns(epoch_number, start..end, Some(&txn))
    }

    /// Gets all historic transactions for a given epoch.
    fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        let txn = txn_option.or_new(&self.db);

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            epoch_number,
        ));

        self.get_historic_txns(epoch_number, 0..tree.num_leaves() as u32, Some(&txn))
    }

    /// Returns the number of historic transactions for a given epoch.
    fn num_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        let txn = txn_option.or_new(&self.db);

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            epoch_number,
        ));

        tree.num_leaves()
    }

    /// Gets the number of historic transactions within an epoch that occurred before the given
    /// block (inclusive).
    fn num_epoch_transactions_before(
        &self,
        mut block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        let txn = txn_option.or_new(&self.db);

        // Find the index of the last transaction that occurred before the given block.
        let epoch_number = Policy::epoch_at(block_number);
        loop {
            // If we switched epochs, this epoch is empty.
            if Policy::epoch_at(block_number) != epoch_number {
                break 0;
            }
            // If start != end, this is the last block that contained transactions.
            let (start, end) = self.get_indexes_for_block(block_number, Some(&txn));
            if start != end {
                break end as usize;
            }
            // If we have reached the beginning of the chain, this epoch is empty.
            if block_number == 0 {
                break 0;
            }
            block_number -= 1;
        }
    }

    /// Gets all historic transactions within an epoch that occurred after the given block.
    fn get_epoch_transactions_after(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        let txn = txn_option.or_new(&self.db);

        // Get history tree for given epoch.
        let epoch_number = Policy::epoch_at(block_number);
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            epoch_number,
        ));

        // Return early if there are no leaves in the HistoryTree for the given epoch.
        let num_leaves = tree.num_leaves() as u32;
        if num_leaves == 0 {
            return vec![];
        }

        // Find the index of the first transaction to return.
        let start_idx = self.num_epoch_transactions_before(block_number, Some(&txn)) as u32;
        self.get_historic_txns(epoch_number, start_idx..num_leaves, Some(&txn))
    }

    /// Returns the `chunk_index`th chunk of size `chunk_size` for a given epoch.
    /// The return value consists of a vector of all the historic transactions in that chunk
    /// and a proof for these in the MMR.
    /// The `verifier_block_number` is the block the chunk proof should be verified against.
    /// That means that no leaf beyond this block is returned and that the proof should be
    /// verified with the history root from this block.
    fn prove_chunk(
        &self,
        epoch_number: u32,
        verifier_block_number: u32,
        chunk_size: usize,
        chunk_index: usize,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoryTreeChunk> {
        let txn = txn_option.or_new(&self.db);

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            epoch_number,
        ));

        // Calculate number of nodes in the verifier's history tree.
        let leaf_count = self.length_at(verifier_block_number, Some(&txn)) as usize;
        let number_of_nodes = leaf_number_to_index(leaf_count);

        // Calculate chunk boundaries.
        let start = cmp::min(chunk_size * chunk_index, leaf_count);
        // Do not go beyond the verifier's block.
        let end = cmp::min(start + chunk_size, leaf_count);

        // TODO: Setting `assume_previous` to false allows the proofs to be verified independently.
        //  This, however, increases the size of the proof. We might change this in the future.
        let proof = tree
            .prove_range(start..end, Some(number_of_nodes), false)
            .ok()?;

        // Get each historic transaction from the tree.
        let hist_txs = self.get_historic_txns(epoch_number, start as u32..end as u32, Some(&txn));

        Some(HistoryTreeChunk {
            proof,
            history: hist_txs,
        })
    }

    /// Creates a new history tree from chunks and returns the root hash.
    fn tree_from_chunks(
        &self,
        epoch_number: u32,
        chunks: Vec<(Vec<HistoricTransaction>, RangeProof<Blake2bHash>)>,
        txn: &mut MdbxWriteTransaction,
    ) -> Result<Blake2bHash, MMRError> {
        // Get partial history tree for given epoch.
        let mut tree = PartialMerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_table,
            txn,
            epoch_number,
        ));

        // Push all proofs into the history tree and remember all leaves.
        let mut all_leaves = Vec::with_capacity(
            chunks.len() * chunks.first().map(|(leaves, _)| leaves.len()).unwrap_or(0),
        );
        for (mut leaves, proof) in chunks {
            tree.push_proof(proof, &leaves)?;
            all_leaves.append(&mut leaves);
        }

        // Calculate the root once the tree is complete.
        if !tree.is_finished() {
            return Err(MMRError::IncompleteProof);
        }

        let root = tree.get_root()?;

        let mut cursor = WriteTransaction::dup_cursor(txn, &self.hist_tx_table);
        // Then add all transactions to the database as the tree is finished.
        for (leaf_index, hist_tx) in all_leaves.iter().enumerate() {
            let value = IndexedTransaction {
                index: leaf_index as u32,
                value: hist_tx.clone(),
            };
            cursor.append(&epoch_number, &value);

            self.validity_store
                .add_transaction(txn, hist_tx.block_number, hist_tx.tx_hash());

            txn.put(
                &self.last_leaf_table,
                &hist_tx.block_number,
                &(leaf_index as u32),
            );
        }

        Ok(root)
    }

    /// Returns the block number of the last leaf in the history store
    fn get_last_leaf_block_number(&self, txn_option: Option<&MdbxReadTransaction>) -> Option<u32> {
        let txn = txn_option.or_new(&self.db);

        // Seek to the last leaf index of the block, if it exists.
        let mut cursor = txn.cursor(&self.last_leaf_table);
        cursor.last().map(|(key, _)| key)
    }

    /// Check whether an equivocation proof at a given equivocation locator has
    /// already been included.
    fn has_equivocation_proof(
        &self,
        locator: EquivocationLocator,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> bool {
        let hash = HistoricTransactionData::Equivocation(EquivocationEvent { locator })
            .hash::<Blake2bHash>()
            .into();
        self.validity_store.has_transaction(txn_option, &hash)
    }

    fn prove_num_leaves(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<SizeProof<Blake2bHash, HistoricTransaction>, MMRError> {
        let txn = txn_option.or_new(&self.db);

        // Get the history tree.
        let epoch_number = Policy::epoch_at(block_number);
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_table,
            &txn,
            epoch_number,
        ));

        let f = |leaf_index| self.get_historic_tx(epoch_number, leaf_index as u32, txn_option);

        // Calculate number of nodes in the verifier's history tree.
        let leaf_count = self.length_at(block_number, Some(&txn)) as usize;
        let number_of_nodes = leaf_number_to_index(leaf_count);

        tree.prove_num_leaves(f, Some(number_of_nodes))
    }

    fn add_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &nimiq_block::Block,
        inherents: Vec<Inherent>,
    ) -> Option<(Blake2bHash, u64)> {
        match block {
            nimiq_block::Block::Macro(macro_block) => {
                // Store the the inherents into the History tree.
                let hist_txs = HistoricTransaction::from(
                    self.network_id,
                    macro_block.header.block_number,
                    macro_block.header.timestamp,
                    vec![],
                    inherents,
                    vec![],
                );

                self.add_to_history(txn, macro_block.block_number(), &hist_txs)
            }
            nimiq_block::Block::Micro(micro_block) => {
                // Get the body of the block.
                let body = micro_block
                    .body
                    .as_ref()
                    .expect("Block body must be present");

                // Store the transactions and the inherents into the History tree.
                let hist_txs = HistoricTransaction::from(
                    self.network_id,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                    body.transactions.clone(),
                    inherents,
                    body.equivocation_proofs
                        .iter()
                        .map(|proof| proof.locator())
                        .collect(),
                );

                self.add_to_history(txn, micro_block.block_number(), &hist_txs)
            }
        }
    }

    fn remove_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &MicroBlock,
        inherents: Vec<Inherent>,
    ) -> Option<u64> {
        let body = block.body.as_ref().unwrap();

        // Remove the transactions from the History tree. For this you only need to calculate the
        // number of transactions that you want to remove.
        let num_txs = HistoricTransaction::count(
            body.transactions.len(),
            &inherents,
            body.equivocation_proofs
                .iter()
                .map(|proof| proof.locator())
                .collect(),
        );

        let (_, total_size) = self
            .remove_partial_history(txn, block.epoch_number(), num_txs)
            .expect("Failed to remove partial history");

        self.validity_store
            .delete_block_transactions(txn, block.block_number());

        Some(total_size)
    }

    fn history_store_range(&self, txn_option: Option<&MdbxReadTransaction>) -> (u32, u32) {
        let txn = txn_option.or_new(&self.db);

        let mut cursor = txn.cursor(&self.last_leaf_table);

        let first = cursor.first().unwrap_or_default().0;
        let last = cursor.last().unwrap_or_default().0;

        (first, last)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_database::mdbx::MdbxDatabase;
    use nimiq_keys::Address;
    use nimiq_primitives::{coin::Coin, networks::NetworkId};
    use nimiq_test_log::test;
    use nimiq_transaction::{
        historic_transaction::{JailEvent, PenalizeEvent, RewardEvent},
        ExecutedTransaction, ForkLocator, Transaction as BlockchainTransaction,
    };

    use super::*;

    #[test]
    fn prove_num_leaves_works() {
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        let mut txn = env.write_transaction();
        let history_root_initial = history_store.get_history_tree_root(0, Some(&txn)).unwrap();

        let size_proof = history_store
            .prove_num_leaves(0, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root_initial));
        assert_eq!(size_proof.size(), 0);

        let size_proof = history_store
            .prove_num_leaves(100, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root_initial));
        assert_eq!(size_proof.size(), 0);

        // Create historic transactions.
        let ext_0 = create_transaction(3, 0);
        let ext_1 = create_transaction(4, 1);
        let ext_2 = create_transaction(5, 2);
        let ext_3 = create_transaction(8, 3);

        // Add first historic transaction to History Store.
        let (history_root0, _) = history_store.add_to_history(&mut txn, 3, &[ext_0]).unwrap();

        let size_proof = history_store
            .prove_num_leaves(0, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root_initial));
        assert_eq!(size_proof.size(), 0);

        let size_proof = history_store
            .prove_num_leaves(100, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root0));
        assert_eq!(size_proof.size(), 1);

        // Add remaining historic transaction to History Store.
        let (history_root1, _) = history_store.add_to_history(&mut txn, 4, &[ext_1]).unwrap();
        let (history_root2, _) = history_store.add_to_history(&mut txn, 5, &[ext_2]).unwrap();
        let (history_root3, _) = history_store.add_to_history(&mut txn, 8, &[ext_3]).unwrap();

        // Prove number of leaves.
        let size_proof = history_store
            .prove_num_leaves(2, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root_initial));
        assert_eq!(size_proof.size(), 0);

        let size_proof = history_store
            .prove_num_leaves(3, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root0));
        assert_eq!(size_proof.size(), 1);

        let size_proof = history_store
            .prove_num_leaves(4, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root1));
        assert_eq!(size_proof.size(), 2);

        let size_proof = history_store
            .prove_num_leaves(5, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root2));
        assert_eq!(size_proof.size(), 3);

        let size_proof = history_store
            .prove_num_leaves(8, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root3));
        assert_eq!(size_proof.size(), 4);

        let size_proof = history_store
            .prove_num_leaves(100, Some(&txn))
            .expect("Should be able to prove number of leaves");
        assert!(size_proof.verify(&history_root3));
        assert_eq!(size_proof.size(), 4);
    }

    #[test]
    fn length_at_works() {
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let ext_0 = create_transaction(1, 0);
        let ext_1 = create_transaction(3, 1);
        let ext_2 = create_transaction(7, 2);
        let ext_3 = create_transaction(8, 3);

        let hist_txs = vec![ext_0, ext_1, ext_2, ext_3];

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, 8, &hist_txs);

        // Verify method works.
        assert_eq!(history_store.length_at(0, Some(&txn)), 0);
        assert_eq!(history_store.length_at(1, Some(&txn)), 1);
        assert_eq!(history_store.length_at(3, Some(&txn)), 2);
        assert_eq!(history_store.length_at(5, Some(&txn)), 2);
        assert_eq!(history_store.length_at(7, Some(&txn)), 3);
        assert_eq!(history_store.length_at(8, Some(&txn)), 4);
        assert_eq!(history_store.length_at(9, Some(&txn)), 4);
    }

    #[test]
    fn transaction_in_validity_window_works() {
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let ext_0 = create_transaction(Policy::genesis_block_number() + 1, 0);
        let ext_1 = create_transaction(Policy::genesis_block_number() + 1, 1);

        let hist_txs = vec![ext_0.clone(), ext_1.clone()];

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 1, &hist_txs);

        // Those transactions should be part of the valitidy window
        assert_eq!(
            history_store.tx_in_validity_window(&ext_0.tx_hash(), Some(&txn)),
            true
        );

        assert_eq!(
            history_store.tx_in_validity_window(&ext_1.tx_hash(), Some(&txn)),
            true
        );

        // Now keep pushing transactions to the history store until we are past the transaction validity window
        let validity_window_blocks = Policy::transaction_validity_window_blocks();

        let mut txn_hashes = vec![];

        for bn in 2..validity_window_blocks + 10 {
            let historic_txn = create_transaction(Policy::genesis_block_number() + bn, bn as u64);
            txn_hashes.push(historic_txn.tx_hash());
            history_store.add_to_history(
                &mut txn,
                Policy::genesis_block_number() + bn,
                &vec![historic_txn],
            );
        }

        // Since we are past the txn in validity window, the first two transaction should no longer be in it
        assert_eq!(
            history_store.tx_in_validity_window(&ext_0.tx_hash(), Some(&txn)),
            false
        );

        assert_eq!(
            history_store.tx_in_validity_window(&ext_1.tx_hash(), Some(&txn)),
            false
        );

        for txn_hash in &txn_hashes[..8] {
            assert_eq!(
                history_store.tx_in_validity_window(txn_hash, Some(&txn)),
                false
            );
        }

        for txn_hash in &txn_hashes[8..] {
            assert_eq!(
                history_store.tx_in_validity_window(txn_hash, Some(&txn)),
                true
            );
        }
    }

    #[test]
    fn get_root_from_hist_txs_works() {
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let hist_txs = gen_hist_txs();

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 0, &hist_txs[..3]);
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs[3..]);

        // Verify method works.
        let real_root_0 =
            history_store.get_history_tree_root(Policy::genesis_block_number() + 0, Some(&txn));
        let calc_root_0 = HistoryStore::_root_from_hist_txs(&hist_txs[..3]);

        assert_eq!(real_root_0, calc_root_0);

        let real_root_1 =
            history_store.get_history_tree_root(Policy::genesis_block_number() + 1, Some(&txn));
        let calc_root_1 = HistoryStore::_root_from_hist_txs(&hist_txs[3..]);

        assert_eq!(real_root_1, calc_root_1);
    }

    #[test]
    fn get_block_transactions_works() {
        let genesis_block_number = Policy::genesis_block_number();
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let hist_txs = gen_hist_txs();

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 0, &hist_txs[..3]);
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs[3..]);

        // Verify method works. Note that the block transactions are returned in the same
        // order they were inserted.
        let query_0 = history_store.get_block_transactions(genesis_block_number, Some(&txn));

        assert!(!query_0[0].is_not_basic());
        assert_eq!(query_0[0].block_number, genesis_block_number);
        assert_eq!(
            query_0[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(0),
        );

        assert!(!query_0[1].is_not_basic());
        assert_eq!(query_0[1].block_number, genesis_block_number);
        assert_eq!(
            query_0[1].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(1),
        );

        assert!(query_0[2].is_not_basic());
        assert_eq!(query_0[2].block_number, genesis_block_number);
        assert_eq!(
            query_0[2].unwrap_reward().value,
            Coin::from_u64_unchecked(2),
        );

        let query_1 = history_store.get_block_transactions(1 + genesis_block_number, Some(&txn));

        assert!(!query_1[0].is_not_basic());
        assert_eq!(query_1[0].block_number, 1 + genesis_block_number);
        assert_eq!(
            query_1[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(3),
        );

        assert!(query_1[1].is_not_basic());
        assert_eq!(query_1[1].block_number, 1 + genesis_block_number);
        assert_eq!(
            query_1[1].unwrap_reward().value,
            Coin::from_u64_unchecked(4),
        );

        let query_2 = history_store.get_block_transactions(2 + genesis_block_number, Some(&txn));

        assert!(!query_2[0].is_not_basic());
        assert_eq!(query_2[0].block_number, 2 + genesis_block_number);
        assert_eq!(
            query_2[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(5),
        );

        assert!(!query_2[1].is_not_basic());
        assert_eq!(query_2[1].block_number, 2 + genesis_block_number);
        assert_eq!(
            query_2[1].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(6),
        );

        assert!(query_2[2].is_not_basic());
        assert_eq!(query_2[2].block_number, 2 + genesis_block_number);
        assert_eq!(
            query_2[2].unwrap_reward().value,
            Coin::from_u64_unchecked(7),
        );

        assert!(query_2[3].is_not_basic());
        assert_eq!(query_2[3], create_jail_inherent(query_2[3].block_number));

        assert!(query_2[4].is_not_basic());
        assert_eq!(
            query_2[4],
            create_penalize_inherent(query_2[4].block_number)
        );

        assert!(query_2[5].is_not_basic());
        assert_eq!(
            query_2[5],
            create_equivocation_inherent(query_2[5].block_number)
        );

        // Remove historic transactions from History Store.
        history_store.remove_partial_history(&mut txn, 0, 2);
        history_store.remove_partial_history(&mut txn, 1, 3);

        // Verify method works. Note that the block transactions are returned in the same
        // order they were inserted.
        let query_0 = history_store.get_block_transactions(genesis_block_number, Some(&txn));

        assert_eq!(query_0.len(), 1);
        assert!(!query_0[0].is_not_basic());
        assert_eq!(query_0[0].block_number, genesis_block_number);
        assert_eq!(
            query_0[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(0),
        );

        let query_1 = history_store.get_block_transactions(1 + genesis_block_number, Some(&txn));

        assert_eq!(query_1.len(), 2);
        assert!(!query_1[0].is_not_basic());
        assert_eq!(query_1[0].block_number, 1 + genesis_block_number);
        assert_eq!(
            query_1[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(3),
        );

        assert!(query_1[1].is_not_basic());
        assert_eq!(query_1[1].block_number, 1 + genesis_block_number);
        assert_eq!(
            query_1[1].unwrap_reward().value,
            Coin::from_u64_unchecked(4),
        );

        let query_2: Vec<HistoricTransaction> =
            history_store.get_block_transactions(2 + genesis_block_number, Some(&txn));

        assert_eq!(query_2.len(), 3);
        assert!(!query_2[0].is_not_basic());
        assert_eq!(query_2[0].block_number, 2 + genesis_block_number);
        assert_eq!(
            query_2[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(5),
        );

        assert!(!query_2[1].is_not_basic());
        assert_eq!(query_2[1].block_number, 2 + genesis_block_number);
        assert_eq!(
            query_2[1].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(6),
        );

        assert!(query_2[2].is_not_basic());
        assert_eq!(query_2[2].block_number, 2 + genesis_block_number);
        assert_eq!(
            query_2[2].unwrap_reward().value,
            Coin::from_u64_unchecked(7),
        );

        // Remove all historic transactions from the last epoch.
        history_store.remove_partial_history(&mut txn, 1, 3);
        let query_2: Vec<HistoricTransaction> =
            history_store.get_block_transactions(2 + genesis_block_number, Some(&txn));

        assert!(query_2.is_empty());
    }

    #[test]
    fn get_epoch_transactions_works() {
        let genesis_block_number = Policy::genesis_block_number();
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let hist_txs = gen_hist_txs();

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 0, &hist_txs[..3]);
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs[3..]);

        // Verify method works.
        let query = history_store.get_epoch_transactions(0, Some(&txn));

        assert!(!query[0].is_not_basic());
        assert_eq!(query[0].block_number, genesis_block_number);
        assert_eq!(
            query[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(0)
        );

        assert!(!query[1].is_not_basic());
        assert_eq!(query[1].block_number, genesis_block_number);
        assert_eq!(
            query[1].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(1)
        );

        assert!(query[2].is_not_basic());
        assert_eq!(query[2].block_number, genesis_block_number);
        assert_eq!(query[2].unwrap_reward().value, Coin::from_u64_unchecked(2));

        let query = history_store.get_epoch_transactions(1, Some(&txn));

        assert!(!query[0].is_not_basic());
        assert_eq!(query[0].block_number, 1 + genesis_block_number);
        assert_eq!(
            query[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(3)
        );

        assert!(query[1].is_not_basic());
        assert_eq!(query[1].block_number, 1 + genesis_block_number);
        assert_eq!(query[1].unwrap_reward().value, Coin::from_u64_unchecked(4));

        assert!(!query[2].is_not_basic());
        assert_eq!(query[2].block_number, 2 + genesis_block_number);
        assert_eq!(
            query[2].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(5)
        );

        assert!(!query[3].is_not_basic());
        assert_eq!(query[3].block_number, 2 + genesis_block_number);
        assert_eq!(
            query[3].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(6)
        );

        assert!(query[4].is_not_basic());
        assert_eq!(query[4].block_number, 2 + genesis_block_number);
        assert_eq!(query[4].unwrap_reward().value, Coin::from_u64_unchecked(7));

        // Remove historic transactions to History Store.
        history_store.remove_partial_history(&mut txn, 1, 3);

        // Verify method works.
        let query = history_store.get_epoch_transactions(0, Some(&txn));

        assert!(!query[0].is_not_basic());
        assert_eq!(query[0].block_number, genesis_block_number);
        assert_eq!(
            query[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(0)
        );

        assert!(!query[1].is_not_basic());
        assert_eq!(query[1].block_number, genesis_block_number);
        assert_eq!(
            query[1].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(1)
        );

        assert!(query[2].is_not_basic());
        assert_eq!(query[2].block_number, genesis_block_number);
        assert_eq!(query[2].unwrap_reward().value, Coin::from_u64_unchecked(2));

        let query = history_store.get_epoch_transactions(1, Some(&txn));

        assert!(!query[0].is_not_basic());
        assert_eq!(query[0].block_number, 1 + genesis_block_number);
        assert_eq!(
            query[0].unwrap_basic().get_raw_transaction().value,
            Coin::from_u64_unchecked(3)
        );

        assert!(query[1].is_not_basic());
        assert_eq!(query[1].block_number, 1 + genesis_block_number);
        assert_eq!(query[1].unwrap_reward().value, Coin::from_u64_unchecked(4));
    }

    #[test]
    fn get_num_historic_transactions_works() {
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let hist_txs = gen_hist_txs();

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 0, &hist_txs[..3]);
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs[3..]);

        // Verify method works.
        assert_eq!(history_store.num_epoch_transactions(0, Some(&txn)), 3);

        assert_eq!(history_store.num_epoch_transactions(1, Some(&txn)), 8);

        // Remove historic transactions to History Store.
        history_store.remove_partial_history(&mut txn, 1, 3);

        // Verify method works.
        assert_eq!(history_store.num_epoch_transactions(0, Some(&txn)), 3);

        assert_eq!(history_store.num_epoch_transactions(1, Some(&txn)), 5);
    }

    #[test]
    fn get_indexes_for_block_works() {
        let genesis_block_number = Policy::genesis_block_number();
        // Initialize History Store.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);
        let mut txn = env.write_transaction();

        for i in genesis_block_number..=(16 * Policy::blocks_per_batch() + genesis_block_number) {
            if Policy::is_macro_block_at(i) {
                let hist_txs = vec![
                    create_reward_inherent(i, 1),
                    create_reward_inherent(i, 2),
                    create_reward_inherent(i, 3),
                    create_reward_inherent(i, 4),
                ];

                history_store.add_to_history(&mut txn, i, &hist_txs);
            }
        }

        assert_eq!(
            history_store.get_indexes_for_block(genesis_block_number, Some(&txn)),
            (0, 4)
        );

        for i in 1..=16 {
            assert_eq!(
                history_store.get_indexes_for_block(32 * i + genesis_block_number, Some(&txn)),
                ((i - 1) % 4 * 4, ((i - 1) % 4 + 1) * 4)
            );
        }

        // Remove historic transactions from History Store.
        for i in (1..=16).rev() {
            history_store.remove_partial_history(
                &mut txn,
                Policy::epoch_at(i * Policy::blocks_per_batch() + genesis_block_number),
                4,
            );

            for j in 1..i {
                assert_eq!(
                    history_store.get_indexes_for_block(32 * j + genesis_block_number, Some(&txn)),
                    ((j - 1) % 4 * 4, ((j - 1) % 4 + 1) * 4)
                );
            }
        }
    }

    fn create_reward_inherent(block: u32, value: u64) -> HistoricTransaction {
        let reward_address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Reward(RewardEvent {
                validator_address: Address::burn_address(),
                reward_address,
                value: Coin::from_u64_unchecked(value),
            }),
        }
    }

    fn create_jail_inherent(block: u32) -> HistoricTransaction {
        let jail_address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Jail(JailEvent {
                validator_address: jail_address,
                slots: 1..3,
                offense_event_block: block,
                new_epoch_slot_range: None,
            }),
        }
    }

    fn create_penalize_inherent(block: u32) -> HistoricTransaction {
        let jail_address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Penalize(PenalizeEvent {
                validator_address: jail_address,
                offense_event_block: block,
                slot: 0,
            }),
        }
    }

    fn create_equivocation_inherent(block: u32) -> HistoricTransaction {
        let address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Equivocation(EquivocationEvent {
                locator: EquivocationLocator::Fork(ForkLocator {
                    validator_address: address,
                    block_number: block,
                }),
            }),
        }
    }

    fn create_transaction(block: u32, value: u64) -> HistoricTransaction {
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Basic(ExecutedTransaction::Ok(
                BlockchainTransaction::new_basic(
                    Address::from_user_friendly_address(
                        "NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF",
                    )
                    .unwrap(),
                    Address::burn_address(),
                    Coin::from_u64_unchecked(value),
                    Coin::from_u64_unchecked(0),
                    0,
                    NetworkId::UnitAlbatross,
                ),
            )),
        }
    }

    fn gen_hist_txs() -> Vec<HistoricTransaction> {
        let genesis_block_number = Policy::genesis_block_number();
        let ext_0 = create_transaction(genesis_block_number + 0, 0);
        let ext_1 = create_transaction(genesis_block_number + 0, 1);
        let ext_2 = create_reward_inherent(genesis_block_number + 0, 2);

        let ext_3 = create_transaction(genesis_block_number + 1, 3);
        let ext_4 = create_reward_inherent(genesis_block_number + 1, 4);

        let ext_5 = create_transaction(genesis_block_number + 2, 5);
        let ext_6 = create_transaction(genesis_block_number + 2, 6);
        let ext_7 = create_reward_inherent(genesis_block_number + 2, 7);
        let ext_8 = create_jail_inherent(genesis_block_number + 2);
        let ext_9 = create_penalize_inherent(genesis_block_number + 2);
        let ext_10 = create_equivocation_inherent(genesis_block_number + 2);

        vec![
            ext_0, ext_1, ext_2, ext_3, ext_4, ext_5, ext_6, ext_7, ext_8, ext_9, ext_10,
        ]
    }
}
