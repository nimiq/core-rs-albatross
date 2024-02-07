use nimiq_database::{TransactionProxy, WriteTransactionProxy};
use nimiq_transaction::historic_transaction::HistoricTransaction;

use super::{interface::HistoryInterface, validity_store::ValidityStore};

/// The LightHistoryStore is essentially an MMRthat only stores peaks.
/// It also contains a validity store, the is used to keep track of which
/// transactions have been included in the validity window.
pub struct LightHistoryStore {
    _validity_store: ValidityStore,
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
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Option<nimiq_hash::Blake2bHash> {
        todo!()
    }

    fn clear(&self, _txn: &mut WriteTransactionProxy) {
        todo!()
    }

    fn length_at(&self, _block_number: u32, _txn_option: Option<&TransactionProxy>) -> u32 {
        todo!()
    }

    fn total_len_at_epoch(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> usize {
        todo!()
    }

    fn add_to_history(
        &self,
        _txn: &mut WriteTransactionProxy,
        _epoch_number: u32,
        _hist_txs: &[HistoricTransaction],
    ) -> Option<(nimiq_hash::Blake2bHash, u64)> {
        todo!()
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
        todo!()
    }

    fn num_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> usize {
        todo!()
    }

    fn get_final_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<nimiq_transaction::historic_transaction::HistoricTransaction> {
        todo!()
    }

    fn get_number_final_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> usize {
        todo!()
    }

    fn get_nonfinal_epoch_transactions(
        &self,
        _epoch_number: u32,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<nimiq_transaction::historic_transaction::HistoricTransaction> {
        todo!()
    }

    fn get_tx_hashes_by_address(
        &self,
        _address: &nimiq_keys::Address,
        _max: u16,
        _txn_option: Option<&TransactionProxy>,
    ) -> Vec<nimiq_hash::Blake2bHash> {
        todo!()
    }

    fn prove(
        &self,
        _epoch_number: u32,
        _hashes: Vec<&nimiq_hash::Blake2bHash>,
        _verifier_state: Option<usize>,
        _txn_option: Option<&TransactionProxy>,
    ) -> Option<nimiq_transaction::history_proof::HistoryTreeProof> {
        todo!()
    }

    fn prove_chunk(
        &self,
        _epoch_number: u32,
        _verifier_block_number: u32,
        _chunk_size: usize,
        _chunk_index: usize,
        _txn_option: Option<&TransactionProxy>,
    ) -> Option<crate::HistoryTreeChunk> {
        todo!()
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
        todo!()
    }

    fn get_last_leaf_block_number(&self, _txn_option: Option<&TransactionProxy>) -> Option<u32> {
        todo!()
    }

    fn has_equivocation_proof(
        &self,
        _locator: nimiq_transaction::EquivocationLocator,
        _txn_option: Option<&TransactionProxy>,
    ) -> bool {
        todo!()
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
        todo!()
    }
}
