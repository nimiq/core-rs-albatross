use std::cmp;

use merkle_mountain_range::error::Error as MMRError;
use merkle_mountain_range::hash::Hash as MMRHash;
use merkle_mountain_range::mmr::partial::PartialMerkleMountainRange;
use merkle_mountain_range::mmr::proof::RangeProof;
use merkle_mountain_range::mmr::MerkleMountainRange;
use merkle_mountain_range::store::memory::MemoryStore;

use nimiq_database::{Database, Environment, ReadTransaction, Transaction, WriteTransaction};
use nimiq_hash::Blake2bHash;

use crate::history_store::mmr_store::MMRStore;
use crate::history_store::{
    ExtendedTransaction, HistoryTreeChunk, HistoryTreeHash, HistoryTreeProof,
};
use nimiq_database::cursor::ReadCursor;

/// A struct that contains databases to store history trees (which are Merkle Mountain Ranges
/// constructed from the list of extended transactions in an epoch) and extended transactions (which
/// are representations of transactions).
/// The history trees allow a node in possession of a transaction to prove to another node (that
/// only has macro block headers) that that given transaction happened.
#[derive(Debug)]
pub struct HistoryStore {
    env: Environment,
    // A database of all history trees indexed by their epoch number.
    hist_tree_db: Database,
    // A database of all extended transactions indexed by their hash (= leaf hash in the history
    // tree).
    ext_tx_db: Database,
    // A database of all leaf hashes indexed by the hash of the transaction. This way we can start
    // with a transaction hash and find it in the MMR.
    leaf_hash_db: Database,
    // A database of all leaf indexes indexed by the hash of the transaction. This way we can start
    // with a transaction hash and find it in the MMR.
    leaf_idx_db: Database,
    // A database of all leaf hashes indexed by the block number where the transaction appears.
    block_db: Database,
}

impl HistoryStore {
    const HIST_TREE_DB_NAME: &'static str = "HistoryTrees";
    const EXT_TX_DB_NAME: &'static str = "ExtendedTransactions";
    const LEAF_HASH_DB_NAME: &'static str = "LeafHashesByHash";
    const LEAF_IDX_DB_NAME: &'static str = "LeafIndexesByHash";
    const BLOCK_DB_NAME: &'static str = "LeafHashesByBlock";

    /// Creates a new HistoryStore.
    pub fn new(env: Environment) -> Self {
        let hist_tree_db = env.open_database(Self::HIST_TREE_DB_NAME.to_string());
        let ext_tx_db = env.open_database(Self::EXT_TX_DB_NAME.to_string());
        let leaf_hash_db = env.open_database(Self::LEAF_HASH_DB_NAME.to_string());
        let leaf_idx_db = env.open_database(Self::LEAF_IDX_DB_NAME.to_string());
        let block_db = env.open_database(Self::BLOCK_DB_NAME.to_string());

        HistoryStore {
            env,
            hist_tree_db,
            ext_tx_db,
            leaf_hash_db,
            leaf_idx_db,
            block_db,
        }
    }

    /// Add a list of extended transactions to an existing history tree. It returns the root of the
    /// resulting tree.
    pub fn add_to_history(
        &self,
        txn: &mut WriteTransaction,
        epoch_number: u32,
        ext_txs: &[ExtendedTransaction],
    ) -> Option<Blake2bHash> {
        // Get the history tree.
        let mut tree = MerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Append the extended transactions to the history tree and keep the respective leaf indexes.
        let mut leaf_idx = vec![];

        for tx in ext_txs {
            let i = tree.push(tx).ok()?;
            leaf_idx.push(i as u32);
        }

        let root = tree.get_root().ok()?.to_blake2b();

        // Add the extended transactions into the respective database.
        // We need to do this separately due to the borrowing rules of Rust.
        for (tx, i) in ext_txs.iter().zip(leaf_idx.iter()) {
            // The prefix is one because it is a leaf.
            self.put_extended_tx(txn, &tx.hash(1).to_blake2b(), *i, tx);
        }

        // Return the history root.
        Some(root)
    }

    /// Removes a number of extended transactions from an existing history tree. It returns the root
    /// of the resulting tree.
    pub fn remove_partial_history(
        &self,
        txn: &mut WriteTransaction,
        epoch_number: u32,
        num_ext_txs: usize,
    ) -> Option<Blake2bHash> {
        // Get the history tree.
        let mut tree = MerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Get the history root. We need to get it here because of Rust's borrowing rules.
        let root = tree.get_root().ok()?.to_blake2b();

        // Remove all leaves from the history tree and remember the respective hashes and indexes.
        let mut hashes = Vec::with_capacity(num_ext_txs);

        let num_leaves = tree.num_leaves();

        for i in 0..cmp::min(num_ext_txs, num_leaves) {
            let leaf_hash = tree.get_leaf(num_leaves - i - 1).unwrap();
            tree.remove_back().ok()?;
            hashes.push((num_leaves - i - 1, leaf_hash));
        }

        // Remove each of the extended transactions in the history tree from the extended
        // transaction database.
        for (index, hash) in hashes {
            self.remove_extended_tx(txn, &hash.to_blake2b(), index as u32);
        }

        // Return the history root.
        Some(root)
    }

    /// Removes an existing history tree and all the extended transactions that were part of it.
    /// Returns None if there's no history tree corresponding to the given epoch number.
    pub fn remove_history(&self, txn: &mut WriteTransaction, epoch_number: u32) -> Option<()> {
        // Get the history tree.
        let mut tree = MerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Remove all leaves from the history tree and remember the respective hashes.
        let mut hashes = Vec::with_capacity(tree.num_leaves());

        for i in (0..tree.num_leaves()).rev() {
            let leaf_hash = tree.get_leaf(i).unwrap();
            tree.remove_back().ok()?;
            hashes.push((i, leaf_hash));
        }

        // Remove each of the extended transactions in the history tree from the extended
        // transaction database.
        for (index, hash) in hashes {
            self.remove_extended_tx(txn, &hash.to_blake2b(), index as u32);
        }

        Some(())
    }

    /// Gets the history tree root for a given epoch.
    pub fn get_history_tree_root(
        &self,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Blake2bHash> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Get the history tree.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Return the history root.
        Some(tree.get_root().ok()?.to_blake2b())
    }

    /// Calculates the history tree root from a vector of extended transactions. It doesn't use the
    /// database, it is just used to check the correctness of the history root when syncing.
    pub fn get_root_from_ext_txs(ext_txs: &[ExtendedTransaction]) -> Option<Blake2bHash> {
        // Create a new history tree.
        let mut tree = MerkleMountainRange::new(MemoryStore::new());

        // Append the extended transactions to the history tree.
        for tx in ext_txs {
            tree.push(tx).ok()?;
        }

        // Return the history root.
        Some(tree.get_root().ok()?.to_blake2b())
    }

    /// Gets an extended transaction given its hash.
    pub fn get_ext_tx_by_hash(
        &self,
        hash: &Blake2bHash,
        txn_option: Option<&Transaction>,
    ) -> Vec<ExtendedTransaction> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Get leaf hash.
        let leaf_hashes = self.get_leaf_hash(hash, Some(txn));

        // Get extended transactions.
        let mut ext_txs = vec![];

        for hash in leaf_hashes {
            ext_txs.push(self.get_extended_tx(&hash, Some(txn)).unwrap());
        }

        ext_txs
    }

    /// Gets all extended transactions for a given block number.
    pub fn get_block_transactions(
        &self,
        block_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Vec<ExtendedTransaction> {
        // Get the leaf hashes at this height.
        let leaf_hashes = self.get_leaf_hash_by_block(block_number, txn_option);

        // Get each extended transaction.
        let mut ext_txs = vec![];

        for hash in leaf_hashes {
            ext_txs.push(self.get_extended_tx(&hash, txn_option).unwrap());
        }

        ext_txs
    }

    /// Gets all extended transactions for a given batch.
    pub fn get_batch_transactions(
        &self,
        batch_number: u32,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Vec<ExtendedTransaction> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Get each extended transaction from the tree.
        let mut ext_txs = vec![];

        for i in 0..tree.num_leaves() {
            let leaf_hash = tree.get_leaf(i).unwrap();
            let ext_tx = self
                .get_extended_tx(&leaf_hash.to_blake2b(), Some(txn))
                .unwrap();

            // Only include extended transactions that match the provided batch number.
            if batch_number == nimiq_primitives::policy::batch_at(ext_tx.block_number) {
                ext_txs.push(ext_tx);
            }
        }

        ext_txs
    }

    /// Gets all extended transactions for a given epoch.
    pub fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Vec<ExtendedTransaction> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Get each extended transaction from the tree.
        let mut ext_txs = vec![];

        for i in 0..tree.num_leaves() {
            let leaf_hash = tree.get_leaf(i).unwrap();
            ext_txs.push(
                self.get_extended_tx(&leaf_hash.to_blake2b(), Some(txn))
                    .unwrap(),
            );
        }

        ext_txs
    }

    /// Returns the number of extended transactions for a given epoch.
    pub fn get_num_extended_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> usize {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        tree.num_leaves()
    }

    /// Returns a proof for transactions with the given hashes. The proof also includes the extended
    /// transactions.
    pub fn prove(
        &self,
        epoch_number: u32,
        hashes: Vec<&Blake2bHash>,
        txn_option: Option<&Transaction>,
    ) -> Option<HistoryTreeProof> {
        // Get the leaf indexes.
        let mut positions = vec![];

        for hash in hashes {
            let mut indices = self
                .get_leaf_index(hash, txn_option)
                .iter()
                .map(|i| *i as usize)
                .collect();

            positions.append(&mut indices)
        }

        self.prove_with_position(epoch_number, positions, txn_option)
    }

    /// Returns a proof for all the extended transactions at the given positions (leaf indexes). The
    /// proof also includes the extended transactions.
    pub fn prove_with_position(
        &self,
        epoch_number: u32,
        positions: Vec<usize>,
        txn_option: Option<&Transaction>,
    ) -> Option<HistoryTreeProof> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Create Merkle proof.
        let proof = tree.prove(&positions).ok()?;

        // Get each extended transaction from the tree.
        let mut ext_txs = vec![];

        for i in &positions {
            let leaf_hash = tree.get_leaf(*i).unwrap();
            ext_txs.push(
                self.get_extended_tx(&leaf_hash.to_blake2b(), Some(txn))
                    .unwrap(),
            );
        }

        Some(HistoryTreeProof {
            proof,
            positions,
            history: ext_txs,
        })
    }

    /// Returns the `chunk_index`th chunk of size `chunk_size` for a given epoch.
    /// The return value consists of a vector of all the extended transactions in that chunk
    /// and a proof for these in the MMR.
    pub fn prove_chunk(
        &self,
        epoch_number: u32,
        chunk_size: usize,
        chunk_index: usize,
        txn_option: Option<&Transaction>,
    ) -> Option<HistoryTreeChunk> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Get history tree for given epoch.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        let start = cmp::min(chunk_size * chunk_index, tree.num_leaves());
        let end = cmp::min(start + chunk_size, tree.num_leaves());

        // TODO: Setting `assume_previous` to false allows the proofs to be verified independently.
        //  This, however, increases the size of the proof. We might change this in the future.
        let proof = tree.prove_range(start..end, false).ok()?;

        // Get each extended transaction from the tree.
        let mut ext_txs = vec![];

        for i in start..end {
            let leaf_hash = tree.get_leaf(i).unwrap();
            ext_txs.push(
                self.get_extended_tx(&leaf_hash.to_blake2b(), Some(txn))
                    .unwrap(),
            );
        }

        Some(HistoryTreeChunk {
            proof,
            history: ext_txs,
        })
    }

    /// Creates a new history tree from chunks and returns the root hash.
    pub fn tree_from_chunks(
        &self,
        epoch_number: u32,
        chunks: Vec<(Vec<ExtendedTransaction>, RangeProof<HistoryTreeHash>)>,
        txn: &mut WriteTransaction,
    ) -> Result<Blake2bHash, MMRError> {
        // Get partial history tree for given epoch.
        let mut tree = PartialMerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_db,
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

        let root = tree.get_root()?.to_blake2b();

        // Then add all transactions to the database as the tree is finished.
        for (i, leaf) in all_leaves.iter().enumerate() {
            // The prefix is one because it is a leaf.
            self.put_extended_tx(txn, &leaf.hash(1).to_blake2b(), i as u32, &leaf);
        }

        Ok(root)
    }

    /// Gets an extended transaction by its hash. Note that this hash is the leaf hash (see MMRHash)
    /// of the transaction, not a simple Blake2b hash of the transaction.
    fn get_extended_tx(
        &self,
        leaf_hash: &Blake2bHash,
        txn_option: Option<&Transaction>,
    ) -> Option<ExtendedTransaction> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        txn.get(&self.ext_tx_db, leaf_hash)
    }

    /// Inserts a extended transaction into the extended transaction database.
    fn put_extended_tx(
        &self,
        txn: &mut WriteTransaction,
        leaf_hash: &Blake2bHash,
        leaf_index: u32,
        ext_tx: &ExtendedTransaction,
    ) {
        txn.put_reserve(&self.ext_tx_db, leaf_hash, ext_tx);

        let tx_hash = ext_tx.tx_hash();

        let block_number = ext_tx.block_number;

        txn.put(&self.leaf_hash_db, &tx_hash, leaf_hash);
        txn.put(&self.leaf_idx_db, &tx_hash, &leaf_index);
        txn.put(&self.block_db, &block_number, leaf_hash);
    }

    /// Removes a extended transaction from the extended transaction database.
    fn remove_extended_tx(
        &self,
        txn: &mut WriteTransaction,
        leaf_hash: &Blake2bHash,
        leaf_index: u32,
    ) {
        // Get the transaction first.
        let tx_opt: Option<ExtendedTransaction> = txn.get(&self.ext_tx_db, leaf_hash);

        let ext_tx = match tx_opt {
            Some(v) => v,
            None => return,
        };

        let tx_hash = ext_tx.tx_hash();

        let block_number = ext_tx.block_number;

        txn.remove_item(&self.leaf_hash_db, &tx_hash, leaf_hash);
        txn.remove_item(&self.leaf_idx_db, &tx_hash, &leaf_index);
        txn.remove_item(&self.block_db, &block_number, leaf_hash);

        txn.remove(&self.ext_tx_db, leaf_hash);
    }

    /// Returns a vector containing all leaf hashes corresponding to the given transaction hash.
    fn get_leaf_hash(
        &self,
        tx_hash: &Blake2bHash,
        txn_option: Option<&Transaction>,
    ) -> Vec<Blake2bHash> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut leaf_hashes = vec![];

        // Seek to the first leaf hash at the given transaction hash.
        let mut cursor = txn.cursor(&self.leaf_hash_db);
        let leaf_hash = match cursor.seek_key::<Blake2bHash, Blake2bHash>(tx_hash) {
            None => return leaf_hashes,
            Some(v) => v,
        };

        leaf_hashes.push(leaf_hash);

        loop {
            // Get next leaf hash.
            match cursor.next_duplicate::<Blake2bHash, Blake2bHash>() {
                Some((_, leaf_hash)) => leaf_hashes.push(leaf_hash),
                None => break,
            };
        }

        leaf_hashes
    }

    /// Returns a vector containing all leaf indexes corresponding to the given transaction hash.
    fn get_leaf_index(&self, tx_hash: &Blake2bHash, txn_option: Option<&Transaction>) -> Vec<u32> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut leaf_indices = vec![];

        // Seek to the first leaf index at the given transaction hash.
        let mut cursor = txn.cursor(&self.leaf_idx_db);
        let leaf_index = match cursor.seek_key::<Blake2bHash, u32>(tx_hash) {
            None => return leaf_indices,
            Some(v) => v,
        };

        leaf_indices.push(leaf_index);

        loop {
            // Get next leaf index.
            match cursor.next_duplicate::<Blake2bHash, u32>() {
                Some((_, leaf_index)) => leaf_indices.push(leaf_index),
                None => break,
            };
        }

        leaf_indices
    }

    /// Returns a vector containing all leaf hashes corresponding to the given block number.
    fn get_leaf_hash_by_block(
        &self,
        block_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Vec<Blake2bHash> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut leaf_hashes = vec![];

        // Seek to the first leaf hash at the given block number.
        let mut cursor = txn.cursor(&self.block_db);
        let leaf_hash = match cursor.seek_key::<u32, Blake2bHash>(&block_number) {
            None => return leaf_hashes,
            Some(v) => v,
        };

        leaf_hashes.push(leaf_hash);

        loop {
            // Get next leaf hash.
            match cursor.next_duplicate::<u32, Blake2bHash>() {
                Some((_, leaf_hash)) => leaf_hashes.push(leaf_hash),
                None => break,
            };
        }

        leaf_hashes
    }
}
