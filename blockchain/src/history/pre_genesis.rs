use std::cmp;

use nimiq_block::{Block, MicroBlock};
use nimiq_database::mdbx::{MdbxReadTransaction, MdbxWriteTransaction};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_mmr::{
    error::Error,
    mmr::proof::{RangeProof, SizeProof},
};
use nimiq_primitives::policy::Policy;
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, RawTransactionHash},
    history_proof::HistoryTreeProof,
    inherent::Inherent,
    EquivocationLocator,
};

use super::interface::{HistoryIndexInterface, HistoryInterface};

/// A wrapper around two history stores, one for the pre-genesis epoch and one for the main epoch.
#[derive(Debug)]
pub struct HistoryStoreMerger<S: HistoryInterface> {
    /// The pre-genesis history store is read-only.
    /// When populating it, we load it separately.
    pre_genesis: Option<S>,
    /// The main history store.
    main: S,
}

impl<S: HistoryInterface> HistoryStoreMerger<S> {
    pub(crate) fn new(pre_genesis: Option<S>, main: S) -> Self {
        Self { pre_genesis, main }
    }
}

impl<S: HistoryInterface> HistoryInterface for HistoryStoreMerger<S> {
    fn add_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &Block,
        inherents: Vec<Inherent>,
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(
            Policy::epoch_at(block.block_number()),
            0,
            "Epoch 0 is pre-genesis"
        );
        self.main.add_block(txn, block, inherents)
    }

    fn remove_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &MicroBlock,
        inherents: Vec<Inherent>,
    ) -> Option<u64> {
        assert_ne!(
            Policy::epoch_at(block.block_number()),
            0,
            "Epoch 0 is pre-genesis"
        );
        self.main.remove_block(txn, block, inherents)
    }

    fn remove_history(&self, txn: &mut MdbxWriteTransaction, epoch_number: u32) -> Option<()> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main.remove_history(txn, epoch_number)
    }

    fn get_history_tree_root(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<Blake2bHash> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()?
                .get_history_tree_root(block_number, None)
        } else {
            self.main.get_history_tree_root(block_number, txn_option)
        }
    }

    fn clear(&self, txn: &mut MdbxWriteTransaction) {
        self.main.clear(txn);
    }

    fn length_at(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<u32> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis.as_ref()?.length_at(block_number, None)
        } else {
            self.main.length_at(block_number, txn_option)
        }
    }

    fn total_len_at_epoch(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.total_len_at_epoch(0, None))
                .unwrap_or_default()
        } else {
            self.main.total_len_at_epoch(epoch_number, txn_option)
        }
    }

    fn history_store_range(&self, txn_option: Option<&MdbxReadTransaction>) -> (u32, u32) {
        let (pre_genesis_start, pre_genesis_end) = self
            .pre_genesis
            .as_ref()
            .map(|pre_genesis| pre_genesis.history_store_range(None))
            .unwrap_or_default();
        let (main_start, main_end) = self.main.history_store_range(txn_option);
        (
            cmp::min(pre_genesis_start, main_start),
            cmp::max(pre_genesis_end, main_end),
        )
    }

    fn add_to_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(Policy::epoch_at(block_number), 0, "Epoch 0 is pre-genesis");
        self.main.add_to_history(txn, block_number, hist_txs)
    }

    fn add_to_history_for_epoch(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main
            .add_to_history_for_epoch(txn, epoch_number, block_number, hist_txs)
    }

    fn remove_partial_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        num_hist_txs: usize,
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main
            .remove_partial_history(txn, epoch_number, num_hist_txs)
    }

    fn tx_in_validity_window(
        &self,
        raw_tx_hash: &RawTransactionHash,
        txn_opt: Option<&MdbxReadTransaction>,
    ) -> bool {
        self.main.tx_in_validity_window(raw_tx_hash, txn_opt)
    }

    fn get_block_transactions(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.get_block_transactions(block_number, None))
                .unwrap_or_default()
        } else {
            self.main.get_block_transactions(block_number, txn_option)
        }
    }

    fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.get_epoch_transactions(epoch_number, None))
                .unwrap_or_default()
        } else {
            self.main.get_epoch_transactions(epoch_number, txn_option)
        }
    }

    fn num_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.num_epoch_transactions(epoch_number, None))
                .unwrap_or_default()
        } else {
            self.main.num_epoch_transactions(epoch_number, txn_option)
        }
    }

    fn num_epoch_transactions_before(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.num_epoch_transactions_before(block_number, None))
                .unwrap_or_default()
        } else {
            self.main
                .num_epoch_transactions_before(block_number, txn_option)
        }
    }

    fn get_epoch_transactions_after(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.get_epoch_transactions_after(block_number, None))
                .unwrap_or_default()
        } else {
            self.main
                .get_epoch_transactions_after(block_number, txn_option)
        }
    }

    fn prove_chunk(
        &self,
        epoch_number: u32,
        verifier_block_number: u32,
        chunk_size: usize,
        chunk_index: usize,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<super::HistoryTreeChunk> {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis.as_ref()?.prove_chunk(
                epoch_number,
                verifier_block_number,
                chunk_size,
                chunk_index,
                None,
            )
        } else {
            self.main.prove_chunk(
                epoch_number,
                verifier_block_number,
                chunk_size,
                chunk_index,
                txn_option,
            )
        }
    }

    fn tree_from_chunks(
        &self,
        epoch_number: u32,
        chunks: Vec<(Vec<HistoricTransaction>, RangeProof<Blake2bHash>)>,
        txn: &mut MdbxWriteTransaction,
    ) -> Result<Blake2bHash, Error> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main.tree_from_chunks(epoch_number, chunks, txn)
    }

    fn get_last_leaf_block_number(&self, txn_option: Option<&MdbxReadTransaction>) -> Option<u32> {
        self.main
            .get_last_leaf_block_number(txn_option)
            .or_else(|| self.pre_genesis.as_ref()?.get_last_leaf_block_number(None))
    }

    fn has_equivocation_proof(
        &self,
        locator: EquivocationLocator,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> bool {
        self.main.has_equivocation_proof(locator, txn_option)
    }

    fn prove_num_leaves(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<SizeProof<Blake2bHash, HistoricTransaction>, Error> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .ok_or(Error::EmptyTree)?
                .prove_num_leaves(block_number, None)
        } else {
            self.main.prove_num_leaves(block_number, txn_option)
        }
    }
}

impl<S: HistoryInterface + HistoryIndexInterface> HistoryIndexInterface for HistoryStoreMerger<S> {
    fn get_hist_tx_by_hash(
        &self,
        raw_tx_hash: &Blake2bHash,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoricTransaction> {
        self.main
            .get_hist_tx_by_hash(raw_tx_hash, txn_option)
            .or_else(|| {
                self.pre_genesis
                    .as_ref()?
                    .get_hist_tx_by_hash(raw_tx_hash, None)
            })
    }

    fn get_tx_hashes_by_address(
        &self,
        address: &Address,
        max: u16,
        start_at: Option<Blake2bHash>,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<Blake2bHash> {
        let mut tx_hashes =
            self.main
                .get_tx_hashes_by_address(address, max, start_at.clone(), txn_option);

        // Fill up from pre-genesis if necessary.
        if tx_hashes.len() < max as usize && self.pre_genesis.is_some() {
            // If the transaction hashes are empty, we can start at the given hash
            // because the hash does not seem to be in the main database.
            let pre_genesis_start = if tx_hashes.is_empty() { start_at } else { None };

            let mut pre_genesis_tx_hashes =
                self.pre_genesis.as_ref().unwrap().get_tx_hashes_by_address(
                    address,
                    max - tx_hashes.len() as u16,
                    pre_genesis_start,
                    None,
                );
            tx_hashes.append(&mut pre_genesis_tx_hashes);
        }

        tx_hashes
    }

    fn prove(
        &self,
        epoch_number: u32,
        hashes: Vec<&Blake2bHash>,
        verifier_state: Option<usize>,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoryTreeProof> {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()?
                .prove(epoch_number, hashes, verifier_state, None)
        } else {
            self.main
                .prove(epoch_number, hashes, verifier_state, txn_option)
        }
    }
}
