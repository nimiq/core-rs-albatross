use std::cmp;

use merkle_mountain_range::error::Error as MMRError;
use merkle_mountain_range::hash::Hash as MMRHash;
use merkle_mountain_range::mmr::partial::PartialMerkleMountainRange;
use merkle_mountain_range::mmr::proof::RangeProof;
use merkle_mountain_range::mmr::MerkleMountainRange;
use merkle_mountain_range::store::memory::MemoryStore;

use nimiq_database::{Database, Environment, ReadTransaction, Transaction, WriteTransaction};
use nimiq_hash::{Blake2bHash, Hash};

use crate::history_store::mmr_store::MMRStore;
use crate::history_store::{
    ExtendedTransaction, HistoryTreeChunk, HistoryTreeHash, HistoryTreeProof,
};

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
    // A database of all leaf hashes (only basic transactions, not inherents) indexed by the hash of
    // the transaction. This way we can start with a transaction hash and find it in the MMR.
    tx_hash_db: Database,
}

impl HistoryStore {
    const HIST_TREE_DB_NAME: &'static str = "HistoryTrees";
    const EXT_TX_DB_NAME: &'static str = "ExtendedTransactions";
    const TX_HASH_DB_NAME: &'static str = "TransactionHashes";

    /// Creates a new HistoryStore.
    pub fn new(env: Environment) -> Self {
        let hist_tree_db = env.open_database(Self::HIST_TREE_DB_NAME.to_string());
        let ext_tx_db = env.open_database(Self::EXT_TX_DB_NAME.to_string());
        let tx_hash_db = env.open_database(Self::TX_HASH_DB_NAME.to_string());
        HistoryStore {
            env,
            hist_tree_db,
            ext_tx_db,
            tx_hash_db,
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
        // Add the extended transactions into the respective database.
        // We need to do this separately due to the borrowing rules of Rust.
        for tx in ext_txs {
            // The prefix is one because it is a leaf.
            self.put_extended_tx(txn, &tx.hash(1).to_blake2b(), tx);
        }

        // Get the history tree.
        let mut tree = MerkleMountainRange::new(MMRStore::with_write_transaction(
            &self.hist_tree_db,
            txn,
            epoch_number,
        ));

        // Append the extended transactions to the history tree.
        for tx in ext_txs {
            tree.push(tx).ok()?;
        }

        // Return the history root.
        Some(tree.get_root().ok()?.to_blake2b())
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

        // Remove all leaves from the history tree and remember the respective hashes.
        let mut hashes = Vec::with_capacity(num_ext_txs);
        let num_leaves = tree.num_leaves();
        for i in 0..cmp::min(num_ext_txs, num_leaves) {
            let leaf_hash = tree.get_leaf(num_leaves - i - 1).unwrap();
            tree.remove_back().ok()?;
            hashes.push(leaf_hash);
        }

        // Remove each of the extended transactions in the history tree from the extended
        // transaction database.
        for hash in hashes {
            self.remove_extended_tx(txn, &hash.to_blake2b());
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
            hashes.push(leaf_hash);
        }

        // Remove each of the extended transactions in the history tree from the extended
        // transaction database.
        for hash in hashes {
            self.remove_extended_tx(txn, &hash.to_blake2b());
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

    /// Gets a basic transaction (no inherents!) given its hash.
    pub fn get_extended_transaction_by_transaction_hash(
        &self,
        hash: &Blake2bHash,
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

        // Get leaf hash.
        let leaf_hash = self.get_leaf_hash(hash, Some(txn))?;

        // Get extended transaction.
        self.get_extended_tx(&leaf_hash, Some(txn))
    }

    /// Gets all extended transactions for a given epoch.
    pub fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<ExtendedTransaction>> {
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

        Some(ext_txs)
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

    /// Returns a proof for all the extended transactions at the given positions (leaf indexes). The
    /// proof also includes the extended transactions.
    pub fn prove(
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
        for leaf in all_leaves {
            // The prefix is one because it is a leaf.
            self.put_extended_tx(txn, &leaf.hash(1).to_blake2b(), &leaf);
        }

        Ok(root)
    }

    /// Gets an extended transaction by its hash. Note that this hash is the leaf hash (see MMRHash)
    /// of the transaction, not a simple Blake2b hash of the transaction.
    fn get_extended_tx(
        &self,
        hash: &Blake2bHash,
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

        txn.get(&self.ext_tx_db, hash)
    }

    /// Gets a leaf hash from the hash of its transaction (only basic, no inherents!).
    fn get_leaf_hash(
        &self,
        hash: &Blake2bHash,
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

        txn.get(&self.tx_hash_db, hash)
    }

    /// Inserts a extended transaction into the extended transaction database. If it's a basic
    /// transaction, we also update the transaction hash database.
    fn put_extended_tx(
        &self,
        txn: &mut WriteTransaction,
        hash: &Blake2bHash,
        ext_tx: &ExtendedTransaction,
    ) {
        txn.put_reserve(&self.ext_tx_db, hash, ext_tx);

        if !ext_tx.is_inherent() {
            let tx_hash: Blake2bHash = ext_tx.unwrap_basic().hash();
            txn.put(&self.tx_hash_db, &tx_hash, hash)
        }
    }

    /// Removes a extended transaction from the extended transaction database. If it's a basic
    /// transaction, we also update the transaction hash database.
    fn remove_extended_tx(&self, txn: &mut WriteTransaction, hash: &Blake2bHash) {
        // Get the transaction first.
        let tx_opt: Option<ExtendedTransaction> = txn.get(&self.ext_tx_db, hash);

        let ext_tx = match tx_opt {
            Some(v) => v,
            None => return,
        };

        if !ext_tx.is_inherent() {
            let tx_hash: Blake2bHash = ext_tx.unwrap_basic().hash();
            txn.remove(&self.tx_hash_db, &tx_hash)
        }

        txn.remove(&self.ext_tx_db, hash);
    }
}
