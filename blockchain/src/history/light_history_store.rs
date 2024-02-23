use nimiq_database::{
    traits::{Database, WriteTransaction},
    DatabaseProxy, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_mmr::mmr::PeaksMerkleMountainRange;
use nimiq_transaction::historic_transaction::HistoricTransaction;

use super::{interface::HistoryInterface, mmr_store::LightMMRStore, validity_store::ValidityStore};

/// The LightHistoryStore is a simplified version of the history store.
/// Internally it uses a Peaks-only MMR
/// It also contains a validity store, the is used to keep track of which
/// transactions have been included in the validity window.
pub struct LightHistoryStore {
    /// Database handle.
    db: DatabaseProxy,
    /// A database of all history trees indexed by their epoch number.
    hist_tree_table: TableProxy,
    /// The container of all the validity window transaction hashes.
    _validity_store: ValidityStore,
}

impl LightHistoryStore {
    /// Creates a new LightHistoryStore.
    pub fn _new(db: DatabaseProxy) -> Self {
        let hist_tree_table = db.open_table("LightHistoryTrees".to_string());

        LightHistoryStore {
            db,
            hist_tree_table,
            _validity_store: ValidityStore {},
        }
    }
}

impl HistoryInterface for LightHistoryStore {
    fn add_block(
        &self,
        _txn: &mut WriteTransactionProxy,
        _block: &nimiq_block::Block,
    ) -> Option<nimiq_hash::Blake2bHash> {
        todo!()
    }

    fn remove_block(&self, _txn: &mut WriteTransactionProxy, _block_number: u32) -> u64 {
        todo!()
    }

    fn remove_history(&self, _txn: &mut WriteTransactionProxy, _epoch_number: u32) -> Option<()> {
        todo!()
    }

    fn get_history_tree_root(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<nimiq_hash::Blake2bHash> {
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
            epoch_number,
        ));

        // Return the history root.
        tree.get_root().ok()
    }

    fn clear(&self, txn: &mut WriteTransactionProxy) {
        txn.clear_database(&self.hist_tree_table);
    }

    fn length_at(&self, _block_number: u32, _txn_option: Option<&TransactionProxy>) -> u32 {
        todo!()
    }

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
        // Get history tree for given epoch.
        let tree = PeaksMerkleMountainRange::new(LightMMRStore::with_read_transaction(
            &self.hist_tree_table,
            txn,
            epoch_number,
        ));

        // Get the Merkle tree length
        tree.len()
    }

    fn add_to_history(
        &self,
        txn: &mut WriteTransactionProxy,
        epoch_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        // Get the history tree.
        let mut tree = PeaksMerkleMountainRange::new(LightMMRStore::with_write_transaction(
            &self.hist_tree_table,
            txn,
            epoch_number,
        ));

        for tx in hist_txs {
            tree.push(tx).ok()?;
        }

        let root = tree.get_root().ok()?;

        // Return the history root.
        Some((root, 0))
    }

    fn remove_partial_history(
        &self,
        _txn: &mut WriteTransactionProxy,
        _epoch_number: u32,
        _num_hist_txs: usize,
    ) -> Option<(nimiq_hash::Blake2bHash, u64)> {
        None
    }

    fn root_from_hist_txs(
        _hist_txs: &[nimiq_transaction::historic_transaction::HistoricTransaction],
    ) -> Option<nimiq_hash::Blake2bHash> {
        todo!()
    }

    fn get_hist_tx_by_hash(
        &self,
        _tx_hash: &nimiq_hash::Blake2bHash,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<nimiq_transaction::historic_transaction::HistoricTransaction> {
        todo!()
    }

    fn get_block_transactions(
        &self,
        _block_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<nimiq_transaction::historic_transaction::HistoricTransaction> {
        todo!()
    }

    fn get_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<nimiq_transaction::historic_transaction::HistoricTransaction> {
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
    ) -> Vec<nimiq_transaction::historic_transaction::HistoricTransaction> {
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
    ) -> Vec<nimiq_transaction::historic_transaction::HistoricTransaction> {
        unimplemented!()
    }

    fn get_tx_hashes_by_address(
        &self,
        _address: &nimiq_keys::Address,
        _max: u16,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<nimiq_hash::Blake2bHash> {
        unimplemented!()
    }

    fn prove(
        &self,
        _epoch_number: u32,
        _hashes: Vec<&nimiq_hash::Blake2bHash>,
        _verifier_state: Option<usize>,
        _txn_option: Option<&TransactionProxy>,
    ) -> Option<nimiq_transaction::history_proof::HistoryTreeProof> {
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
            Vec<nimiq_transaction::historic_transaction::HistoricTransaction>,
            nimiq_mmr::mmr::proof::RangeProof<nimiq_hash::Blake2bHash>,
        )>,
        _txn: &mut WriteTransactionProxy,
    ) -> Result<nimiq_hash::Blake2bHash, nimiq_mmr::error::Error> {
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
        nimiq_mmr::mmr::proof::SizeProof<
            nimiq_hash::Blake2bHash,
            nimiq_transaction::historic_transaction::HistoricTransaction,
        >,
        nimiq_mmr::error::Error,
    > {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use nimiq_database::{traits::Database, volatile::VolatileDatabase};
    use nimiq_genesis::NetworkId;
    use nimiq_keys::Address;
    use nimiq_primitives::coin::Coin;
    use nimiq_transaction::{
        historic_transaction::{HistoricTransaction, HistoricTransactionData},
        ExecutedTransaction, Transaction as BlockchainTransaction,
    };

    use crate::history::{interface::HistoryInterface, light_history_store::LightHistoryStore};

    #[test]
    fn length_at_works() {
        // Initialize History Store.
        let env = VolatileDatabase::new(20).unwrap();
        let history_store = LightHistoryStore::_new(env.clone());

        // Create historic transactions.
        let ext_0 = create_transaction(1, 0);
        let ext_1 = create_transaction(3, 1);
        let ext_2 = create_transaction(7, 2);
        let ext_3 = create_transaction(8, 3);

        let hist_txs = vec![ext_0, ext_1, ext_2, ext_3];

        // Add historic transactions to History Store.
        let mut txn = env.write_transaction();
        history_store.add_to_history(&mut txn, 1, &hist_txs);
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
