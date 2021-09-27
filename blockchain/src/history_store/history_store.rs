use std::cmp;

use nimiq_account::InherentType;
use nimiq_database::cursor::ReadCursor;
use nimiq_database::{
    Database, DatabaseFlags, Environment, ReadTransaction, Transaction, WriteTransaction,
};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_mmr::error::Error as MMRError;
use nimiq_mmr::hash::Hash as MMRHash;
use nimiq_mmr::mmr::partial::PartialMerkleMountainRange;
use nimiq_mmr::mmr::position::leaf_number_to_index;
use nimiq_mmr::mmr::proof::RangeProof;
use nimiq_mmr::mmr::MerkleMountainRange;
use nimiq_mmr::store::memory::MemoryStore;
use nimiq_primitives::policy;

use crate::history_store::mmr_store::MMRStore;
use crate::history_store::ordered_hash::OrderedHash;
use crate::history_store::{ExtendedTransaction, HistoryTreeChunk, HistoryTreeProof};
use crate::ExtTxData;

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
    // A database of all leaf hashes and indexes indexed by the hash of the transaction. This way we
    // can start with a transaction hash and find it in the MMR.
    tx_hash_db: Database,
    // A database of the last leaf index for each block number.
    last_leaf_db: Database,
    // A database of all transaction (and reward inherent) hashes indexed by their sender and
    // recipient addresses.
    address_db: Database,
}

impl HistoryStore {
    const HIST_TREE_DB_NAME: &'static str = "HistoryTrees";
    const EXT_TX_DB_NAME: &'static str = "ExtendedTransactions";
    const TX_HASH_DB_NAME: &'static str = "LeafHashesByTxHash";
    const LAST_LEAF_DB_NAME: &'static str = "LastLeafIndexesByBlock";
    const ADDRESS_DB_NAME: &'static str = "TxHashesByAddress";

    /// Creates a new HistoryStore.
    pub fn new(env: Environment) -> Self {
        let hist_tree_db = env.open_database(Self::HIST_TREE_DB_NAME.to_string());
        let ext_tx_db = env.open_database(Self::EXT_TX_DB_NAME.to_string());
        let tx_hash_db = env.open_database_with_flags(
            Self::TX_HASH_DB_NAME.to_string(),
            DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_FIXED_SIZE_VALUES,
        );
        let last_leaf_db = env.open_database(Self::LAST_LEAF_DB_NAME.to_string());
        let address_db = env.open_database_with_flags(
            Self::ADDRESS_DB_NAME.to_string(),
            DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_FIXED_SIZE_VALUES,
        );

        HistoryStore {
            env,
            hist_tree_db,
            ext_tx_db,
            tx_hash_db,
            last_leaf_db,
            address_db,
        }
    }

    /// Add a list of extended transactions to an existing history tree. It returns the root of the
    /// resulting tree.
    /// This function assumes that:
    ///     1. The transactions are pushed in increasing block number order.
    ///     2. All the blocks are consecutive.
    ///     3. We only push transactions for one epoch at a time.
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

        let root = tree.get_root().ok()?;

        // Add the extended transactions into the respective database.
        // We need to do this separately due to the borrowing rules of Rust.
        for (tx, i) in ext_txs.iter().zip(leaf_idx.iter()) {
            // The prefix is one because it is a leaf.
            self.put_extended_tx(txn, &tx.hash(1), *i, tx);
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
        let root = tree.get_root().ok()?;

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
            self.remove_extended_tx(txn, &hash, index as u32);
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
            self.remove_extended_tx(txn, &hash, index as u32);
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
        Some(tree.get_root().ok()?)
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
        Some(tree.get_root().ok()?)
    }

    /// Gets an extended transaction given its transaction hash.
    pub fn get_ext_tx_by_hash(
        &self,
        tx_hash: &Blake2bHash,
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

        // Get leaf hash(es).
        let leaves = self.get_leaves_by_tx_hash(tx_hash, Some(txn));

        // Get extended transactions.
        let mut ext_txs = vec![];

        for leaf in leaves {
            ext_txs.push(self.get_extended_tx(&leaf.hash, Some(txn)).unwrap());
        }

        ext_txs
    }

    /// Gets all extended transactions for a given block number.
    /// This method returns the transactions in the same order that they appear in the block.
    pub fn get_block_transactions(
        &self,
        block_number: u32,
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

        // Get the history tree.
        let tree = MerkleMountainRange::new(MMRStore::with_read_transaction(
            &self.hist_tree_db,
            txn,
            policy::epoch_at(block_number),
        ));

        // Get the range of leaf indexes at this height.
        let (start, end) = self.get_indexes_for_block(block_number, Some(txn));

        // Get each extended transaction.
        let mut ext_txs = vec![];

        for i in start..end {
            let leaf_hash = tree.get_leaf(i as usize).unwrap();
            let ext_tx = self.get_extended_tx(&leaf_hash, Some(txn)).unwrap();
            ext_txs.push(ext_tx);
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
            ext_txs.push(self.get_extended_tx(&leaf_hash, Some(txn)).unwrap());
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

    /// Returns a vector containing all transaction (and reward inherents) hashes corresponding to the given
    /// address. It fetches the transactions from most recent to least recent up to the maximum
    /// number given.
    pub fn get_tx_hashes_by_address(
        &self,
        address: &Address,
        max: u16,
        txn_option: Option<&Transaction>,
    ) -> Vec<Blake2bHash> {
        if max == 0 {
            return vec![];
        }

        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut tx_hashes = vec![];

        // Seek to the first transaction hash at the given address. If there's none, stop here.
        let mut cursor = txn.cursor(&self.address_db);

        if cursor.seek_key::<Address, OrderedHash>(address).is_none() {
            return tx_hashes;
        }

        // Then go to the last transaction hash at the given address and add it to the transaction
        // hashes list.
        tx_hashes.push(cursor.last_duplicate::<OrderedHash>().expect("This shouldn't panic since we already verified before that there is at least one transactions at this address!").hash);

        while tx_hashes.len() < max as usize {
            // Get previous transaction hash.
            match cursor.prev_duplicate::<Address, OrderedHash>() {
                Some((_, v)) => tx_hashes.push(v.hash),
                None => break,
            };
        }

        tx_hashes
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
                .get_leaves_by_tx_hash(hash, txn_option)
                .iter()
                .map(|i| i.index as usize)
                .collect();

            positions.append(&mut indices)
        }

        self.prove_with_position(epoch_number, positions, txn_option)
    }

    /// Returns a proof for all the extended transactions at the given positions (leaf indexes). The
    /// proof also includes the extended transactions.
    fn prove_with_position(
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
        let proof = tree.prove(&positions, None).ok()?;

        // Get each extended transaction from the tree.
        let mut ext_txs = vec![];

        for i in &positions {
            let leaf_hash = tree.get_leaf(*i).unwrap();
            ext_txs.push(self.get_extended_tx(&leaf_hash, Some(txn)).unwrap());
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
        verifier_block_number: u32,
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

        // Calculate number of nodes in the verifier's history tree.
        // Leaf indices are 0 based thus the + 1.
        let leaf_count = self
            .get_last_leaf_index_of_block(verifier_block_number, Some(txn))
            .unwrap() as usize
            + 1;
        let number_of_nodes = leaf_number_to_index(leaf_count);

        // Calculate chunk boundaries
        let start = cmp::min(chunk_size * chunk_index, leaf_count);
        let end = cmp::min(start + chunk_size, leaf_count);

        // TODO: Setting `assume_previous` to false allows the proofs to be verified independently.
        //  This, however, increases the size of the proof. We might change this in the future.
        let proof = tree
            .prove_range(start..end, Some(number_of_nodes), false)
            .ok()?;

        // Get each extended transaction from the tree.
        let mut ext_txs = vec![];

        for i in start..end {
            let leaf_hash = tree.get_leaf(i).unwrap();
            ext_txs.push(self.get_extended_tx(&leaf_hash, Some(txn)).unwrap());
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
        chunks: Vec<(Vec<ExtendedTransaction>, RangeProof<Blake2bHash>)>,
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

        let root = tree.get_root()?;

        // Then add all transactions to the database as the tree is finished.
        for (i, leaf) in all_leaves.iter().enumerate() {
            // The prefix is one because it is a leaf.
            self.put_extended_tx(txn, &leaf.hash(1), i as u32, leaf);
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

    /// Inserts a extended transaction into the History Store's transaction databases.
    fn put_extended_tx(
        &self,
        txn: &mut WriteTransaction,
        leaf_hash: &Blake2bHash,
        leaf_index: u32,
        ext_tx: &ExtendedTransaction,
    ) {
        txn.put_reserve(&self.ext_tx_db, leaf_hash, ext_tx);

        let tx_hash = ext_tx.tx_hash();

        txn.put(
            &self.tx_hash_db,
            &tx_hash,
            &OrderedHash {
                index: leaf_index,
                hash: leaf_hash.clone(),
            },
        );

        txn.put(&self.last_leaf_db, &ext_tx.block_number, &leaf_index);

        match &ext_tx.data {
            ExtTxData::Basic(tx) => {
                let index_tx_sender = self.get_last_tx_index_for_address(&tx.sender, Some(txn)) + 1;

                txn.put(
                    &self.address_db,
                    &tx.sender,
                    &OrderedHash {
                        index: index_tx_sender,
                        hash: tx_hash.clone(),
                    },
                );

                let index_tx_recipient =
                    self.get_last_tx_index_for_address(&tx.recipient, Some(txn)) + 1;

                txn.put(
                    &self.address_db,
                    &tx.recipient,
                    &OrderedHash {
                        index: index_tx_recipient,
                        hash: tx_hash,
                    },
                );
            }
            ExtTxData::Inherent(tx) => {
                // We only add reward inherents to the address database.
                if tx.ty == InherentType::Reward {
                    let index_tx_recipient =
                        self.get_last_tx_index_for_address(&tx.target, Some(txn)) + 1;

                    txn.put(
                        &self.address_db,
                        &tx.target,
                        &OrderedHash {
                            index: index_tx_recipient,
                            hash: tx_hash,
                        },
                    );
                }
            }
        }
    }

    /// Removes a extended transaction from the History Store's transaction databases.
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

        txn.remove(&self.ext_tx_db, leaf_hash);

        let tx_hash = ext_tx.tx_hash();

        txn.remove_item(
            &self.tx_hash_db,
            &tx_hash,
            &OrderedHash {
                index: leaf_index,
                hash: leaf_hash.clone(),
            },
        );

        // Check if you are removing the last extended transaction for this block. If yes, completely
        // remove the block, if not just decrement the last leaf index.
        let block_number = ext_tx.block_number;

        let leaf_last_block = match block_number.checked_sub(1) {
            None => None,
            Some(n) => txn.get::<u32, u32>(&self.last_leaf_db, &n),
        };

        if leaf_index == 0 {
            // If the leaf index is already zero, always remove the block.
            txn.remove(&self.last_leaf_db, &block_number);
        } else if policy::epoch_index_at(block_number) == 0
            || leaf_last_block.is_none()
            || leaf_last_block.unwrap() < leaf_index - 1
        {
            // If there's no previous block in this epoch, or if there is but its last leaf is smaller
            // than our resulting one would be, then we decrement the leaf index for the current block.
            txn.put(&self.last_leaf_db, &block_number, &(leaf_index - 1));
        } else {
            // The remaining case is that there is a previous block in this epoch and its last leaf
            // index would conflict with our new last leaf index. So we remove the block.
            txn.remove(&self.last_leaf_db, &block_number);
        }

        match &ext_tx.data {
            ExtTxData::Basic(tx) => {
                let mut cursor = txn.cursor(&self.address_db);

                // Seek to the last transaction hash at the sender's address and
                // go back until you find the correct one.
                let mut sender_value = None;

                if cursor
                    .seek_key::<Address, OrderedHash>(&tx.sender)
                    .is_some()
                {
                    let mut duplicate = cursor.last_duplicate::<OrderedHash>();

                    while let Some(v) = duplicate {
                        if v.hash == tx_hash {
                            sender_value = Some(v);
                            break;
                        }

                        duplicate = cursor
                            .prev_duplicate::<Address, OrderedHash>()
                            .map(|(_, v)| v);
                    }
                }

                // Seek to the last transaction hash at the recipient's address and
                // go back until you find the correct one.
                let mut recipient_value = None;

                if cursor
                    .seek_key::<Address, OrderedHash>(&tx.recipient)
                    .is_some()
                {
                    let mut duplicate = cursor.last_duplicate::<OrderedHash>();

                    while let Some(v) = duplicate {
                        if v.hash == tx_hash {
                            recipient_value = Some(v);
                            break;
                        }

                        duplicate = cursor
                            .prev_duplicate::<Address, OrderedHash>()
                            .map(|(_, v)| v);
                    }
                }

                // Now remove both values. This weird construction is because of Rust's borrowing rules.
                drop(cursor);

                if let Some(v) = sender_value {
                    txn.remove_item(&self.address_db, &tx.sender, &v);
                }

                if let Some(v) = recipient_value {
                    txn.remove_item(&self.address_db, &tx.recipient, &v);
                }
            }
            ExtTxData::Inherent(tx) => {
                if tx.ty == InherentType::Reward {
                    let mut cursor = txn.cursor(&self.address_db);

                    // Seek to the last inherent hash at the target's address and
                    // go back until you find the correct one.
                    let mut recipient_value = None;

                    if cursor
                        .seek_key::<Address, OrderedHash>(&tx.target)
                        .is_some()
                    {
                        let mut duplicate = cursor.last_duplicate::<OrderedHash>();

                        while let Some(v) = duplicate {
                            if v.hash == tx_hash {
                                recipient_value = Some(v);
                                break;
                            }

                            duplicate = cursor
                                .prev_duplicate::<Address, OrderedHash>()
                                .map(|(_, v)| v);
                        }
                    }

                    // Now remove the value. This weird construction is because of Rust's borrowing rules.
                    drop(cursor);

                    if let Some(v) = recipient_value {
                        txn.remove_item(&self.address_db, &tx.target, &v);
                    }
                }
            }
        }
    }

    // Get the maximum leaf index for a given block-height within its epoch.
    pub fn get_last_leaf_index_of_block(
        &self,
        block_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<u32> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        txn.get(&self.last_leaf_db, &block_number)
    }

    /// Returns a vector containing all leaf hashes and indexes corresponding to the given
    /// transaction hash.
    fn get_leaves_by_tx_hash(
        &self,
        tx_hash: &Blake2bHash,
        txn_option: Option<&Transaction>,
    ) -> Vec<OrderedHash> {
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
        let mut cursor = txn.cursor(&self.tx_hash_db);
        match cursor.seek_key::<Blake2bHash, OrderedHash>(tx_hash) {
            Some(leaf_hash) => leaf_hashes.push(leaf_hash),
            None => return leaf_hashes,
        };

        // Iterate over leaf hashes for this transaction hash.
        while let Some((_, leaf_hash)) = cursor.next_duplicate::<Blake2bHash, OrderedHash>() {
            leaf_hashes.push(leaf_hash);
        }

        leaf_hashes
    }

    /// Returns the range of leaf indexes corresponding to the given block number.
    fn get_indexes_for_block(
        &self,
        block_number: u32,
        txn_option: Option<&Transaction>,
    ) -> (u32, u32) {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let leaf_last_block = match block_number.checked_sub(1) {
            None => None,
            Some(n) => txn.get::<u32, u32>(&self.last_leaf_db, &n),
        };

        let start = if policy::epoch_index_at(block_number) == 0 || leaf_last_block.is_none() {
            0
        } else {
            leaf_last_block.unwrap() + 1
        };

        let end = match txn.get::<u32, u32>(&self.last_leaf_db, &block_number) {
            None => 0,
            Some(i) => i + 1,
        };

        (start, end)
    }

    /// Returns the index of the last transaction (or reward inherent) associated to the given address.
    fn get_last_tx_index_for_address(
        &self,
        address: &Address,
        txn_option: Option<&Transaction>,
    ) -> u32 {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Seek the first key with the given address.
        let mut cursor = txn.cursor(&self.address_db);

        if cursor.seek_key::<Address, OrderedHash>(address).is_none() {
            return 0;
        }

        // Seek to the last transaction hash at the given address and get its index.
        match cursor.last_duplicate::<OrderedHash>() {
            None => 0,
            Some(v) => v.index,
        }
    }
}

#[cfg(test)]
mod tests {
    use nimiq_account::{Inherent, InherentType};
    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_primitives::coin::Coin;
    use nimiq_primitives::networks::NetworkId;
    use nimiq_transaction::Transaction as BlockchainTransaction;

    use crate::ExtTxData;

    use super::*;

    #[test]
    fn get_root_from_ext_txs_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        // Create extended transactions.
        let ext_txs = gen_ext_txs();

        // Add extended transactions to History Store.
        let mut txn = WriteTransaction::new(&env);
        history_store.add_to_history(&mut txn, 0, &ext_txs[..3]);
        history_store.add_to_history(&mut txn, 1, &ext_txs[3..]);

        // Verify method works.
        let real_root_0 = history_store.get_history_tree_root(0, Some(&txn));
        let calc_root_0 = HistoryStore::get_root_from_ext_txs(&ext_txs[..3]);

        assert_eq!(real_root_0, calc_root_0);

        let real_root_1 = history_store.get_history_tree_root(1, Some(&txn));
        let calc_root_1 = HistoryStore::get_root_from_ext_txs(&ext_txs[3..]);

        assert_eq!(real_root_1, calc_root_1);
    }

    #[test]
    fn get_ext_tx_by_hash_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        // Create extended transactions.
        let ext_txs = gen_ext_txs();

        // Add extended transactions to History Store.
        let mut txn = WriteTransaction::new(&env);
        history_store.add_to_history(&mut txn, 0, &ext_txs[..3]);
        history_store.add_to_history(&mut txn, 1, &ext_txs[3..]);

        // Verify method works.
        assert_eq!(
            history_store.get_ext_tx_by_hash(&ext_txs[0].tx_hash(), Some(&txn))[0]
                .unwrap_basic()
                .value,
            Coin::from_u64_unchecked(0),
        );

        assert_eq!(
            history_store.get_ext_tx_by_hash(&ext_txs[2].tx_hash(), Some(&txn))[0]
                .unwrap_inherent()
                .value,
            Coin::from_u64_unchecked(2),
        );

        assert_eq!(
            history_store.get_ext_tx_by_hash(&ext_txs[3].tx_hash(), Some(&txn))[0]
                .unwrap_basic()
                .value,
            Coin::from_u64_unchecked(3),
        );
    }

    #[test]
    fn get_block_transactions_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        // Create extended transactions.
        let ext_txs = gen_ext_txs();

        // Add extended transactions to History Store.
        let mut txn = WriteTransaction::new(&env);
        history_store.add_to_history(&mut txn, 0, &ext_txs[..3]);
        history_store.add_to_history(&mut txn, 1, &ext_txs[3..]);

        // Verify method works. Note that the block transactions are returned in the same
        // order they were inserted.
        let query_0 = history_store.get_block_transactions(0, Some(&txn));

        assert!(!query_0[0].is_inherent());
        assert_eq!(query_0[0].block_number, 0);
        assert_eq!(query_0[0].unwrap_basic().value, Coin::from_u64_unchecked(0));

        assert!(!query_0[1].is_inherent());
        assert_eq!(query_0[1].block_number, 0);
        assert_eq!(query_0[1].unwrap_basic().value, Coin::from_u64_unchecked(1));

        assert!(query_0[2].is_inherent());
        assert_eq!(query_0[2].block_number, 0);
        assert_eq!(
            query_0[2].unwrap_inherent().value,
            Coin::from_u64_unchecked(2)
        );

        let query_1 = history_store.get_block_transactions(1, Some(&txn));

        assert!(!query_1[0].is_inherent());
        assert_eq!(query_1[0].block_number, 1);
        assert_eq!(query_1[0].unwrap_basic().value, Coin::from_u64_unchecked(3));

        assert!(query_1[1].is_inherent());
        assert_eq!(query_1[1].block_number, 1);
        assert_eq!(
            query_1[1].unwrap_inherent().value,
            Coin::from_u64_unchecked(4)
        );

        let query_2 = history_store.get_block_transactions(2, Some(&txn));

        assert!(!query_2[0].is_inherent());
        assert_eq!(query_2[0].block_number, 2);
        assert_eq!(query_2[0].unwrap_basic().value, Coin::from_u64_unchecked(5));

        assert!(!query_2[1].is_inherent());
        assert_eq!(query_2[1].block_number, 2);
        assert_eq!(query_2[1].unwrap_basic().value, Coin::from_u64_unchecked(6));

        assert!(query_2[2].is_inherent());
        assert_eq!(query_2[2].block_number, 2);
        assert_eq!(
            query_2[2].unwrap_inherent().value,
            Coin::from_u64_unchecked(7)
        );

        // Remove extended transactions to History Store.
        history_store.remove_partial_history(&mut txn, 0, 2);
        history_store.remove_partial_history(&mut txn, 1, 3);

        // Verify method works. Note that the block transactions are returned in the same
        // order they were inserted.
        let query_0 = history_store.get_block_transactions(0, Some(&txn));

        assert!(!query_0[0].is_inherent());
        assert_eq!(query_0[0].block_number, 0);
        assert_eq!(query_0[0].unwrap_basic().value, Coin::from_u64_unchecked(0));

        let query_1 = history_store.get_block_transactions(1, Some(&txn));

        assert!(!query_1[0].is_inherent());
        assert_eq!(query_1[0].block_number, 1);
        assert_eq!(query_1[0].unwrap_basic().value, Coin::from_u64_unchecked(3));

        assert!(query_1[1].is_inherent());
        assert_eq!(query_1[1].block_number, 1);
        assert_eq!(
            query_1[1].unwrap_inherent().value,
            Coin::from_u64_unchecked(4)
        );

        let query_2 = history_store.get_block_transactions(2, Some(&txn));

        assert!(query_2.is_empty());
    }

    #[test]
    fn get_epoch_transactions_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        // Create extended transactions.
        let ext_txs = gen_ext_txs();

        // Add extended transactions to History Store.
        let mut txn = WriteTransaction::new(&env);
        history_store.add_to_history(&mut txn, 0, &ext_txs[..3]);
        history_store.add_to_history(&mut txn, 1, &ext_txs[3..]);

        // Verify method works.
        let query = history_store.get_epoch_transactions(0, Some(&txn));

        assert!(!query[0].is_inherent());
        assert_eq!(query[0].block_number, 0);
        assert_eq!(query[0].unwrap_basic().value, Coin::from_u64_unchecked(0));

        assert!(!query[1].is_inherent());
        assert_eq!(query[1].block_number, 0);
        assert_eq!(query[1].unwrap_basic().value, Coin::from_u64_unchecked(1));

        assert!(query[2].is_inherent());
        assert_eq!(query[2].block_number, 0);
        assert_eq!(
            query[2].unwrap_inherent().value,
            Coin::from_u64_unchecked(2)
        );

        let query = history_store.get_epoch_transactions(1, Some(&txn));

        assert!(!query[0].is_inherent());
        assert_eq!(query[0].block_number, 1);
        assert_eq!(query[0].unwrap_basic().value, Coin::from_u64_unchecked(3));

        assert!(query[1].is_inherent());
        assert_eq!(query[1].block_number, 1);
        assert_eq!(
            query[1].unwrap_inherent().value,
            Coin::from_u64_unchecked(4)
        );

        assert!(!query[2].is_inherent());
        assert_eq!(query[2].block_number, 2);
        assert_eq!(query[2].unwrap_basic().value, Coin::from_u64_unchecked(5));

        assert!(!query[3].is_inherent());
        assert_eq!(query[3].block_number, 2);
        assert_eq!(query[3].unwrap_basic().value, Coin::from_u64_unchecked(6));

        assert!(query[4].is_inherent());
        assert_eq!(query[4].block_number, 2);
        assert_eq!(
            query[4].unwrap_inherent().value,
            Coin::from_u64_unchecked(7)
        );

        // Remove extended transactions to History Store.
        history_store.remove_partial_history(&mut txn, 1, 3);

        // Verify method works.
        let query = history_store.get_epoch_transactions(0, Some(&txn));

        assert!(!query[0].is_inherent());
        assert_eq!(query[0].block_number, 0);
        assert_eq!(query[0].unwrap_basic().value, Coin::from_u64_unchecked(0));

        assert!(!query[1].is_inherent());
        assert_eq!(query[1].block_number, 0);
        assert_eq!(query[1].unwrap_basic().value, Coin::from_u64_unchecked(1));

        assert!(query[2].is_inherent());
        assert_eq!(query[2].block_number, 0);
        assert_eq!(
            query[2].unwrap_inherent().value,
            Coin::from_u64_unchecked(2)
        );

        let query = history_store.get_epoch_transactions(1, Some(&txn));

        assert!(!query[0].is_inherent());
        assert_eq!(query[0].block_number, 1);
        assert_eq!(query[0].unwrap_basic().value, Coin::from_u64_unchecked(3));

        assert!(query[1].is_inherent());
        assert_eq!(query[1].block_number, 1);
        assert_eq!(
            query[1].unwrap_inherent().value,
            Coin::from_u64_unchecked(4)
        );
    }

    #[test]
    fn get_num_extended_transactions_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        // Create extended transactions.
        let ext_txs = gen_ext_txs();

        // Add extended transactions to History Store.
        let mut txn = WriteTransaction::new(&env);
        history_store.add_to_history(&mut txn, 0, &ext_txs[..3]);
        history_store.add_to_history(&mut txn, 1, &ext_txs[3..]);

        // Verify method works.
        assert_eq!(
            history_store.get_num_extended_transactions(0, Some(&txn)),
            3
        );

        assert_eq!(
            history_store.get_num_extended_transactions(1, Some(&txn)),
            5
        );

        // Remove extended transactions to History Store.
        history_store.remove_partial_history(&mut txn, 1, 3);

        // Verify method works.
        assert_eq!(
            history_store.get_num_extended_transactions(0, Some(&txn)),
            3
        );

        assert_eq!(
            history_store.get_num_extended_transactions(1, Some(&txn)),
            2
        );
    }

    #[test]
    fn get_tx_hashes_by_address_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        // Create extended transactions.
        let ext_txs = gen_ext_txs();

        // Add extended transactions to History Store.
        let mut txn = WriteTransaction::new(&env);
        history_store.add_to_history(&mut txn, 0, &ext_txs[..3]);
        history_store.add_to_history(&mut txn, 1, &ext_txs[3..]);

        // Verify method works.
        let query_1 = history_store.get_tx_hashes_by_address(
            &Address::from_user_friendly_address("NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF")
                .unwrap(),
            99,
            Some(&txn),
        );

        assert_eq!(query_1.len(), 5);
        assert_eq!(query_1[0], ext_txs[6].tx_hash());
        assert_eq!(query_1[1], ext_txs[5].tx_hash());
        assert_eq!(query_1[2], ext_txs[3].tx_hash());
        assert_eq!(query_1[3], ext_txs[1].tx_hash());
        assert_eq!(query_1[4], ext_txs[0].tx_hash());

        let query_2 =
            history_store.get_tx_hashes_by_address(&Address::burn_address(), 2, Some(&txn));

        assert_eq!(query_2.len(), 2);
        assert_eq!(query_2[0], ext_txs[6].tx_hash());
        assert_eq!(query_2[1], ext_txs[5].tx_hash());

        let query_3 = history_store.get_tx_hashes_by_address(
            &Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap(),
            99,
            Some(&txn),
        );

        assert_eq!(query_3.len(), 3);
        assert_eq!(query_3[0], ext_txs[7].tx_hash());
        assert_eq!(query_3[1], ext_txs[4].tx_hash());
        assert_eq!(query_3[2], ext_txs[2].tx_hash());

        let query_4 = history_store.get_tx_hashes_by_address(
            &Address::from_user_friendly_address("NQ28 1U7R M38P GN5A 7J8R GE62 8QS7 PK2S 4S31")
                .unwrap(),
            99,
            Some(&txn),
        );

        assert_eq!(query_4.len(), 0);
    }

    #[test]
    fn prove_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        // Create extended transactions.
        let ext_txs = gen_ext_txs();

        // Add extended transactions to History Store.
        let mut txn = WriteTransaction::new(&env);
        history_store.add_to_history(&mut txn, 0, &ext_txs[..3]);
        history_store.add_to_history(&mut txn, 1, &ext_txs[3..]);

        // Verify method works.
        let root = history_store.get_history_tree_root(0, Some(&txn)).unwrap();

        let proof = history_store
            .prove(
                0,
                vec![&ext_txs[0].tx_hash(), &ext_txs[2].tx_hash()],
                Some(&txn),
            )
            .unwrap();

        assert_eq!(proof.positions.len(), 2);
        assert_eq!(proof.positions[0], 0);
        assert_eq!(proof.positions[1], 2);

        assert_eq!(proof.history.len(), 2);
        assert_eq!(proof.history[0].tx_hash(), ext_txs[0].tx_hash());
        assert_eq!(proof.history[1].tx_hash(), ext_txs[2].tx_hash());

        assert!(proof.verify(root).unwrap());

        let root = history_store.get_history_tree_root(1, Some(&txn)).unwrap();

        let proof = history_store
            .prove(
                1,
                vec![
                    &ext_txs[3].tx_hash(),
                    &ext_txs[4].tx_hash(),
                    &ext_txs[6].tx_hash(),
                ],
                Some(&txn),
            )
            .unwrap();

        assert_eq!(proof.positions.len(), 3);
        assert_eq!(proof.positions[0], 0);
        assert_eq!(proof.positions[1], 1);
        assert_eq!(proof.positions[2], 3);

        assert_eq!(proof.history.len(), 3);
        assert_eq!(proof.history[0].tx_hash(), ext_txs[3].tx_hash());
        assert_eq!(proof.history[1].tx_hash(), ext_txs[4].tx_hash());
        assert_eq!(proof.history[2].tx_hash(), ext_txs[6].tx_hash());

        assert!(proof.verify(root).unwrap());
    }

    #[test]
    fn prove_empty_tree_works() {
        // Initialize History Store.
        let env = VolatileEnvironment::new(10).unwrap();
        let history_store = HistoryStore::new(env.clone());

        let txn = WriteTransaction::new(&env);

        // Verify method works.
        let root = history_store.get_history_tree_root(0, Some(&txn)).unwrap();

        let proof = history_store.prove(0, vec![], Some(&txn)).unwrap();

        assert_eq!(proof.positions.len(), 0);
        assert_eq!(proof.history.len(), 0);

        assert!(proof.verify(root).unwrap());
    }

    fn create_inherent(block: u32, value: u64) -> ExtendedTransaction {
        ExtendedTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: ExtTxData::Inherent(Inherent {
                ty: InherentType::Reward,
                target: Address::from_user_friendly_address(
                    "NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T",
                )
                .unwrap(),
                value: Coin::from_u64_unchecked(value),
                data: vec![],
            }),
        }
    }

    fn create_transaction(block: u32, value: u64) -> ExtendedTransaction {
        ExtendedTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: ExtTxData::Basic(BlockchainTransaction::new_basic(
                Address::from_user_friendly_address("NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF")
                    .unwrap(),
                Address::burn_address(),
                Coin::from_u64_unchecked(value),
                Coin::from_u64_unchecked(0),
                0,
                NetworkId::Dummy,
            )),
        }
    }

    fn gen_ext_txs() -> Vec<ExtendedTransaction> {
        let ext_0 = create_transaction(0, 0);
        let ext_1 = create_transaction(0, 1);
        let ext_2 = create_inherent(0, 2);
        let ext_3 = create_transaction(1, 3);
        let ext_4 = create_inherent(1, 4);
        let ext_5 = create_transaction(2, 5);
        let ext_6 = create_transaction(2, 6);
        let ext_7 = create_inherent(2, 7);

        vec![ext_0, ext_1, ext_2, ext_3, ext_4, ext_5, ext_6, ext_7]
    }
}
