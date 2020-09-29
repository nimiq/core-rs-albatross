use crate::history_store::extended_transaction::ExtendedTransaction;
use crate::history_store::history_tree::HistoryTree;
use database::{Database, Environment, ReadTransaction, Transaction, WriteTransaction};
use hash::Blake2bHash;
use mmr::hash::Hash as MMRHash;
use mmr::store::memory::MemoryStore;

#[derive(Debug)]
pub struct HistoryStore {
    env: Environment,
    // A database of all history trees indexed by their epoch number.
    hist_tree_db: Database,
    // A database of all extended transactions indexed by their hash.
    ext_tx_db: Database,
}

impl HistoryStore {
    const HIST_TREE_DB_NAME: &'static str = "HistoryTrees";
    const EXT_TX_DB_NAME: &'static str = "ExtendedTransactions";

    pub fn new(env: Environment) -> Self {
        let hist_tree_db = env.open_database(Self::HIST_TREE_DB_NAME.to_string());
        let ext_tx_db = env.open_database(Self::EXT_TX_DB_NAME.to_string());
        HistoryStore {
            env,
            hist_tree_db,
            ext_tx_db,
        }
    }

    pub fn put_history_tree(
        &self,
        txn: &mut WriteTransaction,
        epoch_number: u32,
        history_tree: &HistoryTree,
    ) {
        txn.put_reserve(&self.hist_tree_db, &epoch_number, history_tree);
    }

    pub fn new_history_tree(&self, txn: &mut WriteTransaction, epoch_number: u32) {
        let new_tree = HistoryTree::new(MemoryStore::new());
        self.put_history_tree(txn, epoch_number, &new_tree);
    }

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

    pub fn put_extended_tx(
        &self,
        txn: &mut WriteTransaction,
        hash: &Blake2bHash,
        ext_tx: &ExtendedTransaction,
    ) {
        txn.put_reserve(&self.ext_tx_db, hash, ext_tx);
    }

    pub fn get_extended_tx(
        &self,
        hash: &Blake2bHash,
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
        txn.get(&self.hist_tree_db, hash)
    }

    // Should return root. Function called at the end of every batch.
    pub fn append_history_tree(
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
}
