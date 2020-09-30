use crate::history_store::ExtendedTransaction;
use crate::history_store::HistoryTree;
use database::{Database, Environment, ReadTransaction, Transaction, WriteTransaction};
use hash::Blake2bHash;
use mmr::hash::Hash as MMRHash;

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
}

impl HistoryStore {
    const HIST_TREE_DB_NAME: &'static str = "HistoryTrees";
    const EXT_TX_DB_NAME: &'static str = "ExtendedTransactions";

    /// Creates a new HistoryStore
    pub fn new(env: Environment) -> Self {
        let hist_tree_db = env.open_database(Self::HIST_TREE_DB_NAME.to_string());
        let ext_tx_db = env.open_database(Self::EXT_TX_DB_NAME.to_string());
        HistoryStore {
            env,
            hist_tree_db,
            ext_tx_db,
        }
    }

    /// Creates and stores a new HistoryTree for a given epoch number.
    pub fn new_history(&self, txn: &mut WriteTransaction, epoch_number: u32) {
        self.put_history_tree(txn, epoch_number, &HistoryTree::empty());
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
        let mut tree = self.get_history_tree(epoch_number, None)?;

        // Append the extended transactions to the history tree and store them in the extended
        // transaction database.
        for tx in ext_txs {
            tree.push(tx).ok()?;
            // The prefix is one because it is a leaf.
            self.put_extended_tx(txn, &tx.hash(1).to_blake2b(), tx);
        }

        // Store the updated history tree.
        self.put_history_tree(txn, epoch_number, &tree);

        // Return the history root
        let root = tree.get_root();

        match root {
            Ok(hash) => Some(hash.to_blake2b()),
            Err(_) => None,
        }
    }

    /// Removes an existing history tree and all the extended transactions that were part of it.
    /// Returns None if there's no history tree corresponding to the given epoch number.
    pub fn remove_history(&self, txn: &mut WriteTransaction, epoch_number: u32) -> Option<()> {
        // Get the history tree.
        let tree = self.get_history_tree(epoch_number, None)?;

        // Remove each of the extended transactions in the history tree from the extended
        // transaction database.
        for i in 0..tree.num_leaves() {
            let leaf_hash = tree.get_leaf(i).unwrap();
            self.remove_extended_tx(txn, &leaf_hash.to_blake2b());
        }

        // Remove the history tree.
        self.remove_history_tree(txn, epoch_number);

        Some(())
    }

    /// Gets the history tree for a given epoch.
    pub fn get_history_tree(
        &self,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<HistoryTree> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };
        txn.get(&self.hist_tree_db, &epoch_number)
    }

    /// Gets the history tree root for a given epoch.
    pub fn get_history_tree_root(
        &self,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Blake2bHash> {
        Some(
            self.get_history_tree(epoch_number, txn_option)?
                .get_root()
                .ok()?
                .to_blake2b(),
        )
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
        let tree = self.get_history_tree(epoch_number, None)?;

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

    fn put_history_tree(
        &self,
        txn: &mut WriteTransaction,
        epoch_number: u32,
        history_tree: &HistoryTree,
    ) {
        txn.put_reserve(&self.hist_tree_db, &epoch_number, history_tree);
    }

    fn remove_history_tree(&self, txn: &mut WriteTransaction, epoch_number: u32) {
        txn.remove(&self.hist_tree_db, &epoch_number);
    }

    /// Gets an extended transaction by its hash. Note that this hash is the leaf hash (see MMRHash)
    /// of the transaction, not a simple Blake2b hash of the transaction
    pub fn get_extended_tx(
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

    fn put_extended_tx(
        &self,
        txn: &mut WriteTransaction,
        hash: &Blake2bHash,
        ext_tx: &ExtendedTransaction,
    ) {
        txn.put_reserve(&self.ext_tx_db, hash, ext_tx);
    }

    fn remove_extended_tx(&self, txn: &mut WriteTransaction, hash: &Blake2bHash) {
        txn.remove(&self.ext_tx_db, hash);
    }
}
