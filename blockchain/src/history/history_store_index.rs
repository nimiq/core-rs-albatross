use std::{collections::BTreeMap, ops::Range};

use nimiq_block::MicroBlock;
use nimiq_database::{
    traits::{Database, ReadCursor, ReadTransaction, WriteCursor, WriteTransaction},
    DatabaseProxy, TableFlags, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_mmr::{
    error::Error as MMRError,
    mmr::proof::{RangeProof, SizeProof},
};
use nimiq_primitives::policy::Policy;
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, HistoricTransactionData, RawTransactionHash},
    history_proof::HistoryTreeProof,
    inherent::Inherent,
    EquivocationLocator,
};

use super::{
    interface::HistoryInterface,
    utils::{EpochBasedIndex, IndexedTransaction, OrderedHash},
};
use crate::{history::HistoryTreeChunk, interface::HistoryIndexInterface, HistoryStore};

/// A struct that contains databases to store history indices.
pub struct HistoryStoreIndex {
    /// Database handle.
    db: DatabaseProxy,
    /// A database of all epoch numbers and leaf indices indexed by the hash of the (raw) transaction. This way we
    /// can start with a raw transaction hash and find it in the MMR.
    /// Mapping of raw tx hash to epoch number and leaf index.
    tx_hash_table: TableProxy,
    /// A database of all raw transaction (and reward inherent) hashes indexed by their sender and
    /// recipient addresses.
    address_table: TableProxy,
    /// The history store.
    history_store: HistoryStore,
}

impl HistoryStoreIndex {
    /// `RawTransactonHash` -> `EpochBasedIndex` (`epoch number || leaf_index`)
    const TX_HASH_DB_NAME: &'static str = "LeafIndexByTxHash";
    /// `Address` -> `Vec<OrderedHash>`
    const ADDRESS_DB_NAME: &'static str = "TxHashesByAddress";

    /// Creates a new HistoryStore.
    pub fn new(db: DatabaseProxy, network_id: NetworkId) -> Self {
        let tx_hash_table = db.open_table(Self::TX_HASH_DB_NAME.to_string());
        let address_table = db.open_table_with_flags(
            Self::ADDRESS_DB_NAME.to_string(),
            TableFlags::DUPLICATE_KEYS | TableFlags::DUP_FIXED_SIZE_VALUES,
        );

        let index = HistoryStoreIndex {
            history_store: HistoryStore::new(db.clone(), network_id),
            db,
            tx_hash_table,
            address_table,
        };

        index.rebuild_index_if_necessary();
        index
    }

    /// Rebuild index if necessary.
    fn rebuild_index_if_necessary(&self) {
        let mut txn = self.db.write_transaction();
        let mut hist_tx_cursor = WriteTransaction::cursor(&txn, &self.history_store.hist_tx_table);

        trace!("Check if history index needs to be rebuilt.");
        // Check if last transaction is part of index.
        if let Some((_, hist_tx)) = hist_tx_cursor.last::<u32, IndexedTransaction>() {
            let raw_tx_hash = hist_tx.value.tx_hash();
            if txn
                .get::<RawTransactionHash, EpochBasedIndex>(&self.tx_hash_table, &raw_tx_hash)
                .is_none()
            {
                info!("History index out-of-date. Starting to rebuild index (this can take a long time).");
                self.rebuild_index(&mut txn);
                debug!("Commiting rebuilt index.");
                txn.commit();
                info!("Finished rebuilding history index.")
            }
        }
    }

    fn remove_txns_from_history(
        &self,
        txn: &mut WriteTransactionProxy,
        epoch_number: u32,
        leaf_indices: Range<u32>,
    ) {
        for leaf_index in leaf_indices.clone() {
            let tx_opt = self
                .history_store
                .get_historic_tx(epoch_number, leaf_index, Some(txn));

            let Some(hist_tx) = tx_opt else { continue };

            // Remove it from the transaction hash database.
            let tx_hash = hist_tx.tx_hash();
            let key = EpochBasedIndex::new(epoch_number, leaf_index);
            txn.remove_item(&self.tx_hash_table, &tx_hash, &key);

            let ordered_hash = OrderedHash {
                index: key,
                value: tx_hash.into(),
            };
            match &hist_tx.data {
                HistoricTransactionData::Basic(tx) => {
                    let tx = tx.get_raw_transaction();
                    txn.remove_item(&self.address_table, &tx.sender, &ordered_hash);
                    txn.remove_item(&self.address_table, &tx.recipient, &ordered_hash);
                }
                HistoricTransactionData::Reward(ev) => {
                    txn.remove_item(&self.address_table, &ev.reward_address, &ordered_hash);
                }
                HistoricTransactionData::Equivocation(_)
                | HistoricTransactionData::Penalize(_)
                | HistoricTransactionData::Jail(_) => {}
            }
        }
    }

    /// Inserts a historic transaction into the History Store's transaction databases.
    /// Returns the size of the serialized transaction
    fn put_historic_tx(
        &self,
        hashes: &mut BTreeMap<RawTransactionHash, EpochBasedIndex>,
        addresses: &mut BTreeMap<Address, Vec<OrderedHash>>,
        epoch_number: u32,
        leaf_index: u32,
        hist_tx: &HistoricTransaction,
    ) {
        let key = EpochBasedIndex::new(epoch_number, leaf_index);

        // The raw tx hash corresponds to the hash without the execution result.
        // Thus for basic historic transactions we want discoverability for the raw transaction.
        let raw_tx_hash = hist_tx.tx_hash();
        hashes.insert(raw_tx_hash.clone(), key);

        let ordered_hash = OrderedHash {
            index: key,
            value: raw_tx_hash.into(),
        };
        match &hist_tx.data {
            HistoricTransactionData::Basic(tx) => {
                let tx = tx.get_raw_transaction();
                addresses
                    .entry(tx.sender.clone())
                    .or_default()
                    .push(ordered_hash.clone());
                addresses
                    .entry(tx.recipient.clone())
                    .or_default()
                    .push(ordered_hash);
            }
            HistoricTransactionData::Reward(ev) => {
                // We only add reward inherents to the address database.
                addresses
                    .entry(ev.reward_address.clone())
                    .or_default()
                    .push(ordered_hash);
            }
            // Do not index equivocation or punishments events, since I do not see a use case
            // for this at the time.
            HistoricTransactionData::Equivocation(_)
            | HistoricTransactionData::Penalize(_)
            | HistoricTransactionData::Jail(_) => {}
        }
    }

    /// Returns the epoch index and leaf index corresponding to the given
    /// transaction hash.
    /// The validity window ensures that there is only ever one transaction.
    fn get_leaf_indices_by_tx_hash(
        &self,
        raw_tx_hash: &Blake2bHash,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<EpochBasedIndex> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Iterate leaf hashes at the given transaction hash.
        txn.get(
            &self.tx_hash_table,
            &RawTransactionHash::from(raw_tx_hash.clone()),
        )
    }

    /// Rebuilds the index from scratch.
    /// This is a very expensive operation, which currently is only available in an external binary.
    pub fn rebuild_index(&self, txn: &mut WriteTransactionProxy) {
        // Clear the tables.
        txn.clear_database(&self.tx_hash_table);
        txn.clear_database(&self.address_table);

        // Iterate over all epochs and leafs.
        let mut hashes = BTreeMap::new();
        let mut addresses = BTreeMap::new();
        let cursor = WriteTransaction::cursor(txn, &self.history_store.hist_tx_table);
        debug!("Reading historic transactions.");
        for (key, hist_tx) in cursor.into_iter_start::<EpochBasedIndex, HistoricTransaction>() {
            self.put_historic_tx(
                &mut hashes,
                &mut addresses,
                key.epoch_number,
                key.index,
                &hist_tx,
            );
        }

        // We insert indices by append, which gives us much better performance.
        debug!("Writing transaction hash index");
        let mut hashes_cursor = WriteTransaction::cursor(txn, &self.tx_hash_table);
        for (hash, index) in hashes.iter() {
            hashes_cursor.append(hash, index);
        }

        debug!("Writing address index");
        let mut addresses_cursor = WriteTransaction::cursor(txn, &self.address_table);
        for (address, ordered_hashes) in addresses.iter() {
            for ordered_hash in ordered_hashes.iter() {
                addresses_cursor.append(address, ordered_hash);
            }
        }
    }
}

impl HistoryInterface for HistoryStoreIndex {
    fn add_block(
        &self,
        txn: &mut WriteTransactionProxy,
        block: &nimiq_block::Block,
        inherents: Vec<Inherent>,
    ) -> Option<(Blake2bHash, u64)> {
        match block {
            nimiq_block::Block::Macro(macro_block) => {
                // Store the the inherents into the History tree.
                let hist_txs = HistoricTransaction::from(
                    self.history_store.network_id,
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
                    self.history_store.network_id,
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
        txn: &mut WriteTransactionProxy,
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

        // Remove the block from the validity store (and its corresponding txn hashes)
        self.history_store
            .validity_store
            .delete_block_transactions(txn, block.block_number());

        Some(total_size)
    }

    fn get_history_tree_root(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<Blake2bHash> {
        self.history_store
            .get_history_tree_root(block_number, txn_option)
    }

    fn clear(&self, txn: &mut WriteTransactionProxy) {
        self.history_store.clear(txn);
        txn.clear_database(&self.tx_hash_table);
        txn.clear_database(&self.address_table);
    }

    fn length_at(&self, block_number: u32, txn_option: Option<&TransactionProxy>) -> u32 {
        self.history_store.length_at(block_number, txn_option)
    }

    fn total_len_at_epoch(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> usize {
        self.history_store
            .total_len_at_epoch(epoch_number, txn_option)
    }

    fn history_store_range(&self, txn_option: Option<&TransactionProxy>) -> (u32, u32) {
        self.history_store.history_store_range(txn_option)
    }

    fn tx_in_validity_window(
        &self,
        raw_tx_hash: &Blake2bHash,
        txn_opt: Option<&TransactionProxy>,
    ) -> bool {
        self.history_store
            .tx_in_validity_window(raw_tx_hash, txn_opt)
    }

    fn get_block_transactions(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction> {
        self.history_store
            .get_block_transactions(block_number, txn_option)
    }

    fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction> {
        self.history_store
            .get_epoch_transactions(epoch_number, txn_option)
    }

    fn num_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> usize {
        self.history_store
            .num_epoch_transactions(epoch_number, txn_option)
    }

    fn num_epoch_transactions_before(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> usize {
        self.history_store
            .num_epoch_transactions_before(block_number, txn_option)
    }

    fn get_epoch_transactions_after(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction> {
        self.history_store
            .get_epoch_transactions_after(block_number, txn_option)
    }

    fn prove_chunk(
        &self,
        epoch_number: u32,
        verifier_block_number: u32,
        chunk_size: usize,
        chunk_index: usize,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<HistoryTreeChunk> {
        self.history_store.prove_chunk(
            epoch_number,
            verifier_block_number,
            chunk_size,
            chunk_index,
            txn_option,
        )
    }

    fn tree_from_chunks(
        &self,
        epoch_number: u32,
        chunks: Vec<(Vec<HistoricTransaction>, RangeProof<Blake2bHash>)>,
        txn: &mut WriteTransactionProxy,
    ) -> Result<Blake2bHash, MMRError> {
        self.history_store
            .tree_from_chunks(epoch_number, chunks, txn)
    }

    fn get_last_leaf_block_number(&self, txn_option: Option<&TransactionProxy>) -> Option<u32> {
        self.history_store.get_last_leaf_block_number(txn_option)
    }

    fn has_equivocation_proof(
        &self,
        locator: EquivocationLocator,
        txn_option: Option<&TransactionProxy>,
    ) -> bool {
        self.history_store
            .has_equivocation_proof(locator, txn_option)
    }

    fn prove_num_leaves(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<SizeProof<Blake2bHash, HistoricTransaction>, MMRError> {
        self.history_store
            .prove_num_leaves(block_number, txn_option)
    }

    /// Add a list of historic transactions to an existing history tree. It returns the root of the
    /// resulting tree and the total size of the transactions added.
    /// This function assumes that:
    ///     1. The transactions are pushed in increasing block number order.
    ///     2. All the blocks are consecutive.
    ///     3. We only push transactions for one epoch at a time.
    fn add_to_history(
        &self,
        txn: &mut WriteTransactionProxy,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        if let Some((root, size, leaf_idx)) =
            self.history_store
                .put_historic_txns(txn, block_number, hist_txs)
        {
            let epoch_number = Policy::epoch_at(block_number);
            // Add the historic transactions into the respective database.
            // Sort everything first and then put with a cursor for improved database performance.
            let mut hashes = BTreeMap::new();
            let mut addresses = BTreeMap::new();
            for (tx, i) in hist_txs.iter().zip(leaf_idx.iter()) {
                self.put_historic_tx(&mut hashes, &mut addresses, epoch_number, *i, tx);
            }

            // Put the hashes and addresses into the respective databases.
            let mut hashes_cursor = WriteTransaction::cursor(txn, &self.tx_hash_table);
            for (hash, key) in hashes.iter() {
                hashes_cursor.put(hash, key);
            }
            let mut address_cursor = WriteTransaction::cursor(txn, &self.address_table);
            for (address, keys) in addresses.iter() {
                for ordered_hash in keys.iter() {
                    address_cursor.put(address, ordered_hash);
                }
            }
            return Some((root, size));
        }
        None
    }

    /// Removes a number of historic transactions from an existing history tree. It returns the root
    /// of the resulting tree and the total size of the transactions removed.
    fn remove_partial_history(
        &self,
        txn: &mut WriteTransactionProxy,
        epoch_number: u32,
        num_hist_txs: usize,
    ) -> Option<(Blake2bHash, u64)> {
        let (root, leaf_indices) =
            self.history_store
                .remove_leaves_from_history(txn, epoch_number, Some(num_hist_txs))?;

        // Remove each of the historic transactions in the history tree from the extended
        // transaction database.
        self.remove_txns_from_history(txn, epoch_number, leaf_indices.clone());
        let txns_size =
            self.history_store
                .remove_txns_from_history(txn, epoch_number, leaf_indices);

        // Return the history root.
        Some((root, txns_size))
    }

    /// Removes an existing history tree and all the historic transactions that were part of it.
    /// Returns None if there's no history tree corresponding to the given epoch number.
    fn remove_history(&self, txn: &mut WriteTransactionProxy, epoch_number: u32) -> Option<()> {
        let (_, leaf_indices) =
            self.history_store
                .remove_leaves_from_history(txn, epoch_number, None)?;
        self.remove_txns_from_history(txn, epoch_number, leaf_indices);
        self.history_store
            .remove_epoch_from_history(txn, epoch_number);

        Some(())
    }
}

impl HistoryIndexInterface for HistoryStoreIndex {
    /// Gets an historic transaction given its transaction hash.
    fn get_hist_tx_by_hash(
        &self,
        raw_tx_hash: &Blake2bHash,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<HistoricTransaction> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Get leaf hash(es).
        let leaf = self.get_leaf_indices_by_tx_hash(raw_tx_hash, Some(txn))?;

        self.history_store
            .get_historic_tx(leaf.epoch_number, leaf.index, Some(txn))
    }

    /// Returns a vector containing all transaction (and reward inherents) hashes corresponding to the given
    /// address. It fetches the transactions from most recent to least recent up to the maximum
    /// number given.
    fn get_tx_hashes_by_address(
        &self,
        address: &Address,
        max: u16,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<Blake2bHash> {
        if max == 0 {
            return vec![];
        }

        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        let mut tx_hashes = vec![];

        // Seek to the first transaction hash at the given address. If there's none, stop here.
        let mut cursor = txn.cursor(&self.address_table);

        if cursor.seek_key::<Address, OrderedHash>(address).is_none() {
            return tx_hashes;
        }

        // Then go to the last transaction hash at the given address and add it to the transaction
        // hashes list.
        tx_hashes.push(cursor.last_duplicate::<OrderedHash>().expect("This shouldn't panic since we already verified before that there is at least one transactions at this address!").value);

        while tx_hashes.len() < max as usize {
            // Get previous transaction hash.
            match cursor.prev_duplicate::<Address, OrderedHash>() {
                Some((_, v)) => tx_hashes.push(v.value),
                None => break,
            };
        }

        tx_hashes
    }

    /// Returns a proof for transactions with the given hashes. The proof also includes the extended
    /// transactions.
    /// The verifier state is used for those cases where the verifier might have an incomplete MMR,
    /// for instance this could occur where we want to create transaction inclusion proofs of incomplete epochs.
    fn prove(
        &self,
        epoch_number: u32,
        hashes: Vec<&Blake2bHash>,
        verifier_state: Option<usize>,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<HistoryTreeProof> {
        // Get the leaf indexes.
        let mut positions = vec![];

        for hash in hashes {
            let mut indices = self
                .get_leaf_indices_by_tx_hash(hash, txn_option)
                .iter()
                .map(|i| i.index as usize)
                .collect();

            positions.append(&mut indices)
        }

        self.history_store
            .prove_with_position(epoch_number, positions, verifier_state, txn_option)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_database::volatile::VolatileDatabase;
    use nimiq_primitives::{coin::Coin, networks::NetworkId};
    use nimiq_test_log::test;
    use nimiq_transaction::{
        historic_transaction::{EquivocationEvent, JailEvent, PenalizeEvent, RewardEvent},
        ExecutedTransaction, ForkLocator, Transaction as BlockchainTransaction,
    };

    use super::*;

    #[test]
    fn prove_num_leaves_works() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

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
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

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
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

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
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

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
    fn get_hist_tx_by_hash_works() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let hist_txs = gen_hist_txs();

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 0, &hist_txs[..3]);
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs[3..]);

        let hashes: Vec<_> = hist_txs.iter().map(|hist_tx| hist_tx.tx_hash()).collect();

        // Verify method works.
        assert_eq!(
            history_store
                .get_hist_tx_by_hash(&hashes[0], Some(&txn))
                .unwrap()
                .unwrap_basic()
                .get_raw_transaction()
                .value,
            Coin::from_u64_unchecked(0),
        );

        assert_eq!(
            history_store
                .get_hist_tx_by_hash(&hashes[2], Some(&txn))
                .unwrap()
                .unwrap_reward()
                .value,
            Coin::from_u64_unchecked(2),
        );

        assert_eq!(
            history_store
                .get_hist_tx_by_hash(&hashes[3], Some(&txn))
                .unwrap()
                .unwrap_basic()
                .get_raw_transaction()
                .value,
            Coin::from_u64_unchecked(3),
        );
    }

    #[test]
    fn get_block_transactions_works() {
        let genesis_block_number = Policy::genesis_block_number();
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

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
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

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
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

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
    fn get_tx_hashes_by_address_works() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let hist_txs = gen_hist_txs();

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 0, &hist_txs[..3]);
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs[3..]);

        // Verify method works.
        let query_1 = history_store.get_tx_hashes_by_address(
            &Address::from_user_friendly_address("NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF")
                .unwrap(),
            99,
            Some(&txn),
        );

        let hashes: Vec<_> = hist_txs.iter().map(|hist_tx| hist_tx.tx_hash()).collect();

        assert_eq!(query_1.len(), 5);
        assert_eq!(query_1[0], *hashes[6]);
        assert_eq!(query_1[1], *hashes[5]);
        assert_eq!(query_1[2], *hashes[3]);
        assert_eq!(query_1[3], *hashes[1]);
        assert_eq!(query_1[4], *hashes[0]);

        let query_2 =
            history_store.get_tx_hashes_by_address(&Address::burn_address(), 2, Some(&txn));

        assert_eq!(query_2.len(), 2);
        assert_eq!(query_2[0], *hashes[6]);
        assert_eq!(query_2[1], *hashes[5]);

        let query_3 = history_store.get_tx_hashes_by_address(
            &Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap(),
            99,
            Some(&txn),
        );

        assert_eq!(query_3.len(), 3);
        assert_eq!(query_3[0], *hashes[7]);
        assert_eq!(query_3[1], *hashes[4]);
        assert_eq!(query_3[2], *hashes[2]);

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
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let hist_txs = gen_hist_txs();

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 0, &hist_txs[..3]);
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs[3..]);

        let hashes: Vec<_> = hist_txs.iter().map(|hist_tx| hist_tx.tx_hash()).collect();

        // Verify method works.
        let root = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 0, Some(&txn))
            .unwrap();

        let proof = history_store
            .prove(0, vec![&hashes[0], &hashes[2]], None, Some(&txn))
            .unwrap();

        let proof_hashes: Vec<_> = proof
            .history
            .iter()
            .map(|hist_tx| hist_tx.tx_hash())
            .collect();

        assert_eq!(proof.positions.len(), 2);
        assert_eq!(proof.positions[0], 0);
        assert_eq!(proof.positions[1], 2);

        assert_eq!(proof_hashes.len(), 2);
        assert_eq!(proof_hashes[0], hashes[0]);
        assert_eq!(proof_hashes[1], hashes[2]);

        assert!(proof.verify(root).unwrap());

        let root = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 1, Some(&txn))
            .unwrap();

        let proof = history_store
            .prove(
                1,
                vec![&hashes[3], &hashes[4], &hashes[6]],
                None,
                Some(&txn),
            )
            .unwrap();
        let proof_hashes: Vec<_> = proof
            .history
            .iter()
            .map(|hist_tx| hist_tx.tx_hash())
            .collect();

        assert_eq!(proof.positions.len(), 3);
        assert_eq!(proof.positions[0], 0);
        assert_eq!(proof.positions[1], 1);
        assert_eq!(proof.positions[2], 3);

        assert_eq!(proof_hashes.len(), 3);
        assert_eq!(proof_hashes[0], hashes[3]);
        assert_eq!(proof_hashes[1], hashes[4]);
        assert_eq!(proof_hashes[2], hashes[6]);

        assert!(proof.verify(root).unwrap());
    }

    #[test]
    fn prove_empty_tree_works() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);

        let txn = env.write_transaction();

        // Verify method works.
        let root = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 0, Some(&txn))
            .unwrap();

        let proof = history_store.prove(0, vec![], None, Some(&txn)).unwrap();

        assert_eq!(proof.positions.len(), 0);
        assert_eq!(proof.history.len(), 0);

        assert!(proof.verify(root).unwrap());
    }

    #[test]
    fn get_indexes_for_block_works() {
        let genesis_block_number = Policy::genesis_block_number();
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = HistoryStoreIndex::new(env.clone(), NetworkId::UnitAlbatross);
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
            history_store
                .history_store
                .get_indexes_for_block(genesis_block_number, Some(&txn)),
            (0, 4)
        );

        for i in 1..=16 {
            assert_eq!(
                history_store
                    .history_store
                    .get_indexes_for_block(32 * i + genesis_block_number, Some(&txn)),
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
                    history_store
                        .history_store
                        .get_indexes_for_block(32 * j + genesis_block_number, Some(&txn)),
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
                    NetworkId::Dummy,
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
