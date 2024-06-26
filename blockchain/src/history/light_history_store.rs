use nimiq_block::MicroBlock;
use nimiq_database::{
    traits::{Database, WriteTransaction},
    DatabaseProxy, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_mmr::mmr::PeaksMerkleMountainRange;
use nimiq_primitives::policy::Policy;
use nimiq_transaction::{historic_transaction::HistoricTransaction, inherent::Inherent};

use super::{
    interface::HistoryInterface,
    mmr_store::{get_range, remove_block_from_store, LightMMRStore},
    validity_store::ValidityStore,
};

/// The LightHistoryStore is a simplified version of the history store.
/// Internally it uses a Peaks-only MMR
/// It also contains a validity store, the is used to keep track of which
/// transactions have been included in the validity window.
pub struct LightHistoryStore {
    /// The network ID. It determines if this is the mainnet or one of the testnets.
    pub network_id: NetworkId,
    /// Database handle.
    db: DatabaseProxy,
    /// A database of all history trees indexed by their block number.
    hist_tree_table: TableProxy,
    /// The container of all the validity window transaction hashes.
    validity_store: ValidityStore,
}

impl LightHistoryStore {
    /// Creates a new LightHistoryStore.
    pub fn new(db: DatabaseProxy, network_id: NetworkId) -> Self {
        let hist_tree_table = db.open_table("LightHistoryTrees".to_string());
        let validity_store = ValidityStore::new(db.clone());

        LightHistoryStore {
            db,
            hist_tree_table,
            validity_store,
            network_id,
        }
    }

    fn remove_block_light_history_store(&self, txn: &mut WriteTransactionProxy, block_number: u32) {
        remove_block_from_store(&self.hist_tree_table, txn, block_number);

        self.validity_store
            .delete_block_transactions(txn, block_number);
    }
}

impl HistoryInterface for LightHistoryStore {
    fn add_block(
        &self,
        txn: &mut WriteTransactionProxy,
        block: &nimiq_block::Block,
        inherents: Vec<Inherent>,
    ) -> Option<(Blake2bHash, u64)> {
        match block {
            nimiq_block::Block::Macro(macro_block) => {
                // Store the transactions and the inherents into the History tree.
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
        txn: &mut WriteTransactionProxy,
        block: &MicroBlock,
        _inherents: Vec<Inherent>,
    ) -> Option<u64> {
        remove_block_from_store(&self.hist_tree_table, txn, block.block_number());

        self.validity_store
            .delete_block_transactions(txn, block.block_number());

        // TODO: We dont keep track of the size of the txns that we removed, is this necessary?
        Some(0)
    }

    // Remove the history of the given epoch
    fn remove_history(&self, txn: &mut WriteTransactionProxy, epoch_number: u32) -> Option<()> {
        if epoch_number == 0 {
            return Some(());
        }

        for bn in Policy::first_block_of(epoch_number).unwrap()
            ..(Policy::first_block_of(epoch_number + 1).unwrap() - 1)
        {
            self.remove_block_light_history_store(txn, bn);
        }

        Some(())
    }

    fn get_history_tree_root(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<Blake2bHash> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Get the history tree.
        let tree = PeaksMerkleMountainRange::new(LightMMRStore::with_read_transaction(
            &self.hist_tree_table,
            txn,
            block_number,
        ));

        // Return the history root.
        tree.get_root().ok()
    }

    fn clear(&self, txn: &mut WriteTransactionProxy) {
        txn.clear_database(&self.hist_tree_table);
    }

    fn length_at(&self, block_number: u32, txn_option: Option<&TransactionProxy>) -> u32 {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };
        // Get history tree for given epoch.
        let tree = PeaksMerkleMountainRange::new(LightMMRStore::with_read_transaction(
            &self.hist_tree_table,
            txn,
            block_number,
        ));

        // Get the Merkle tree length
        tree.len() as u32
    }

    fn history_store_range(&self, txn_option: Option<&TransactionProxy>) -> (u32, u32) {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        get_range(&self.hist_tree_table, txn)
    }

    // This returns the total length, in terms of number of leaves of the history store
    // at the given epoch.
    // There are two cases: complete epochs and incomplete epochs.
    // For the first case, we return the length of the tree at the last block of the epoch
    // For the latter, we return the length of the tree at the latest block that was pushed.
    fn total_len_at_epoch(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> usize {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // First we get the first and last block number that have been stored.
        let (first_bn, last_bn) = self.history_store_range(Some(txn));

        if Policy::epoch_at(last_bn) == epoch_number {
            // This is the case for an incomplete epoch, so we get the size of the tree at the last block that was stored
            // Get peaks mmr of the latest block stored
            let tree = PeaksMerkleMountainRange::new(LightMMRStore::with_read_transaction(
                &self.hist_tree_table,
                txn,
                last_bn,
            ));

            // Get the Merkle tree length
            tree.num_leaves()
        } else if first_bn < (Policy::first_block_of(epoch_number + 1).unwrap() - 1) {
            // This is the case of a complete epoch, so we get the size of the tree
            // at the last block of the epoch
            let tree = PeaksMerkleMountainRange::new(LightMMRStore::with_read_transaction(
                &self.hist_tree_table,
                txn,
                Policy::first_block_of(epoch_number + 1).unwrap() - 1,
            ));

            // Get the Merkle tree length
            tree.num_leaves()
        } else {
            log::debug!("Total length at epoch out of bounds!");
            0
        }
    }

    fn add_to_history(
        &self,
        txn: &mut WriteTransactionProxy,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        // Get the history tree.
        let mut tree = PeaksMerkleMountainRange::new(LightMMRStore::with_write_transaction(
            &self.hist_tree_table,
            txn,
            block_number,
        ));

        for tx in hist_txs {
            tree.push(tx).ok()?;
        }

        let root = tree.get_root().ok()?;

        for tx in hist_txs {
            assert!(
                tx.block_number <= block_number
                    && Policy::epoch_at(tx.block_number) == Policy::epoch_at(block_number),
                "Inconsistent transactions when adding to history store (block #{}, tx block #{}).",
                block_number,
                tx.block_number
            );
            self.validity_store
                .add_transaction(txn, tx.block_number, tx.tx_hash().into())
        }

        self.validity_store.update_validity_store(txn, block_number);

        // Return the history root.
        Some((root, 0))
    }

    fn remove_partial_history(
        &self,
        _txn: &mut WriteTransactionProxy,
        _epoch_number: u32,
        _num_hist_txs: usize,
    ) -> Option<(Blake2bHash, u64)> {
        None
    }

    fn tx_in_validity_window(
        &self,
        tx_hash: &Blake2bHash,
        txn_opt: Option<&TransactionProxy>,
    ) -> bool {
        self.validity_store.has_transaction(txn_opt, tx_hash)
    }

    fn get_block_transactions(
        &self,
        _block_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction> {
        todo!()
    }

    fn get_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction> {
        unimplemented!()
    }

    fn num_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> usize {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Get history tree for given epoch.
        let tree = PeaksMerkleMountainRange::new(LightMMRStore::with_read_transaction(
            &self.hist_tree_table,
            txn,
            epoch_number,
        ));

        tree.num_leaves()
    }

    fn get_final_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction> {
        unimplemented!()
    }

    fn get_number_final_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> usize {
        unimplemented!()
    }

    fn get_nonfinal_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction> {
        unimplemented!()
    }

    fn prove_chunk(
        &self,
        _epoch_number: u32,
        _verifier_block_number: u32,
        _chunk_size: usize,
        _chunk_index: usize,
        _txn_option: Option<&TransactionProxy>,
    ) -> Option<crate::HistoryTreeChunk> {
        unimplemented!()
    }

    fn tree_from_chunks(
        &self,
        _epoch_number: u32,
        _chunks: Vec<(
            Vec<HistoricTransaction>,
            nimiq_mmr::mmr::proof::RangeProof<Blake2bHash>,
        )>,
        _txn: &mut WriteTransactionProxy,
    ) -> Result<Blake2bHash, nimiq_mmr::error::Error> {
        unimplemented!()
    }

    fn get_last_leaf_block_number(&self, _txn_option: Option<&TransactionProxy>) -> Option<u32> {
        unimplemented!()
    }

    fn has_equivocation_proof(
        &self,
        _locator: nimiq_transaction::EquivocationLocator,
        _txn_option: Option<&TransactionProxy>,
    ) -> bool {
        unimplemented!()
    }

    fn prove_num_leaves(
        &self,
        _block_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Result<
        nimiq_mmr::mmr::proof::SizeProof<Blake2bHash, HistoricTransaction>,
        nimiq_mmr::error::Error,
    > {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use nimiq_database::{traits::Database, volatile::VolatileDatabase};
    use nimiq_genesis::NetworkId;
    use nimiq_hash::Blake2bHash;
    use nimiq_keys::Address;
    use nimiq_mmr::hash::Merge;
    use nimiq_primitives::{coin::Coin, policy::Policy};
    use nimiq_transaction::{
        historic_transaction::{HistoricTransaction, HistoricTransactionData},
        ExecutedTransaction, Transaction as BlockchainTransaction,
    };

    use crate::history::{interface::HistoryInterface, light_history_store::LightHistoryStore};

    #[test]
    fn length_at_works() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = LightHistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let ext_0 = create_transaction(Policy::genesis_block_number() + 1, 0);
        let ext_1 = create_transaction(Policy::genesis_block_number() + 1, 1);
        let ext_2 = create_transaction(Policy::genesis_block_number() + 2, 2);
        let ext_3 = create_transaction(Policy::genesis_block_number() + 3, 3);
        let ext_4 = create_transaction(Policy::genesis_block_number() + 3, 4);
        let ext_5 = create_transaction(Policy::genesis_block_number() + 3, 5);
        let ext_6 = create_transaction(Policy::genesis_block_number() + 3, 6);
        let ext_7 = create_transaction(Policy::genesis_block_number() + 3, 7);

        let hist_txs = vec![ext_0, ext_1];

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 1, &hist_txs);

        let hist_txs = vec![ext_2];
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs);

        let hist_txs = vec![ext_3, ext_4, ext_5, ext_6, ext_7];
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 3, &hist_txs);

        let len_1 = history_store.length_at(Policy::genesis_block_number() + 1, Some(&txn));
        let len_2 = history_store.length_at(Policy::genesis_block_number() + 2, Some(&txn));
        let len_3 = history_store.length_at(Policy::genesis_block_number() + 3, Some(&txn));

        // Lengths are cumulative because they are block number based
        assert_eq!(len_1, 3);
        assert_eq!(len_2, 4);
        assert_eq!(len_3, 15);
    }

    #[test]
    fn transaction_in_validity_window_works() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = LightHistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let ext_0 = create_transaction(Policy::genesis_block_number() + 1, 0);
        let ext_1 = create_transaction(Policy::genesis_block_number() + 1, 1);

        let hist_txs = vec![ext_0.clone(), ext_1.clone()];

        // Add historic transactions to History Store.
        let mut current_block_number = Policy::genesis_block_number() + 1;
        let validity_window_blocks = Policy::transaction_validity_window_blocks();

        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, current_block_number, &hist_txs);

        // Those transactions should be part of the valitidy window
        // Now keep pushing transactions to the history store until we are past the transaction validity window
        for bn in 2..validity_window_blocks + 2 {
            assert_eq!(
                history_store.tx_in_validity_window(&ext_0.tx_hash(), Some(&txn)),
                true
            );

            assert_eq!(
                history_store.tx_in_validity_window(&ext_1.tx_hash(), Some(&txn)),
                true
            );

            current_block_number += 1;
            let historic_txn = create_transaction(Policy::genesis_block_number() + bn, bn as u64);
            history_store.add_to_history(&mut txn, current_block_number, &vec![historic_txn]);
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

        // Remove the last block and make sure the transactions are in the validity window again
        history_store.remove_block_light_history_store(&mut txn, current_block_number);
        assert_eq!(
            history_store.tx_in_validity_window(&ext_0.tx_hash(), Some(&txn)),
            true
        );

        assert_eq!(
            history_store.tx_in_validity_window(&ext_1.tx_hash(), Some(&txn)),
            true
        );
    }

    #[test]
    fn total_len_at_epoch() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = LightHistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

        // Create historic transactions.
        let ext_0 = create_transaction(Policy::genesis_block_number() + 1, 0);
        let ext_1 = create_transaction(Policy::genesis_block_number() + 1, 1);
        let ext_2 = create_transaction(Policy::genesis_block_number() + 2, 2);
        let ext_3 = create_transaction(Policy::genesis_block_number() + 3, 3);
        let ext_4 = create_transaction(Policy::genesis_block_number() + 3, 4);
        let ext_5 = create_transaction(Policy::genesis_block_number() + 3, 5);
        let ext_6 = create_transaction(Policy::genesis_block_number() + 3, 6);
        let ext_7 = create_transaction(Policy::genesis_block_number() + 3, 7);

        let hist_txs = vec![ext_0, ext_1];

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 1, &hist_txs);

        let hist_txs = vec![ext_2];
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs);

        let hist_txs = vec![ext_3, ext_4, ext_5, ext_6, ext_7];
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 3, &hist_txs);

        let total_length = history_store.total_len_at_epoch(
            Policy::epoch_at(Policy::genesis_block_number() + 1),
            Some(&txn),
        );

        // Lengths are cumulative because they are block number based
        assert_eq!(total_length, 8);
    }

    #[test]
    fn it_can_remove_a_block() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = LightHistoryStore::new(env.clone(), NetworkId::UnitAlbatross);
        let mut txn = env.write_transaction();

        // Create historic transactions for block 1
        let ext_0 = create_transaction(Policy::genesis_block_number() + 1, 0);
        let ext_1 = create_transaction(Policy::genesis_block_number() + 1, 1);
        let hist_txs = vec![ext_0, ext_1];

        // Add historic transactions to History Stor  e.
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 1, &hist_txs);

        // Create historic transactions for block 2
        let ext_0 = create_transaction(Policy::genesis_block_number() + 2, 0);
        let ext_1 = create_transaction(Policy::genesis_block_number() + 2, 1);
        let ext_2 = create_transaction(Policy::genesis_block_number() + 2, 2);
        let hist_txs = vec![ext_0, ext_1, ext_2];

        // Add historic transactions to History Store.
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 2, &hist_txs);

        // Create historic transactions for block 3
        let ext_0 = create_transaction(Policy::genesis_block_number() + 3, 0);
        let ext_1 = create_transaction(Policy::genesis_block_number() + 3, 1);
        let ext_2 = create_transaction(Policy::genesis_block_number() + 3, 2);
        let ext_3 = create_transaction(Policy::genesis_block_number() + 3, 3);
        let hist_txs = vec![ext_0, ext_1, ext_2, ext_3];

        // Add historic transactions to History Store.
        history_store.add_to_history(&mut txn, Policy::genesis_block_number() + 3, &hist_txs);

        let size_1 = history_store.length_at(Policy::genesis_block_number() + 1, Some(&txn));
        let size_2 = history_store.length_at(Policy::genesis_block_number() + 2, Some(&txn));
        let size_3 = history_store.length_at(Policy::genesis_block_number() + 3, Some(&txn));

        assert_eq!(size_1, 3);
        assert_eq!(size_2, 8);
        assert_eq!(size_3, 16);

        let prev_root_1 = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 1, Some(&txn))
            .unwrap();
        let prev_root_2 = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 2, Some(&txn))
            .unwrap();
        let _prev_root_3 = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 3, Some(&txn))
            .unwrap();

        history_store
            .remove_block_light_history_store(&mut txn, Policy::genesis_block_number() + 3);

        let size_1 = history_store.length_at(Policy::genesis_block_number() + 1, Some(&txn));
        let size_2 = history_store.length_at(Policy::genesis_block_number() + 2, Some(&txn));
        let size_3 = history_store.length_at(Policy::genesis_block_number() + 3, Some(&txn));

        assert_eq!(size_1, 3);
        assert_eq!(size_2, 8);
        assert_eq!(size_3, 0);

        let empty_root = Blake2bHash::empty(0);
        let root_1 = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 1, Some(&txn))
            .unwrap();
        let root_2 = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 2, Some(&txn))
            .unwrap();
        let root_3 = history_store
            .get_history_tree_root(Policy::genesis_block_number() + 3, Some(&txn))
            .unwrap();

        assert_eq!(prev_root_1, root_1);
        assert_eq!(prev_root_2, root_2);
        assert_eq!(empty_root, root_3);
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
}
