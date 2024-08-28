use nimiq_block::{Block, MicroBlock};
use nimiq_database::mdbx::{MdbxReadTransaction, MdbxWriteTransaction};
use nimiq_hash::Blake2bHash;
use nimiq_mmr::{
    error::Error as MMRError,
    mmr::proof::{RangeProof, SizeProof},
};
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, RawTransactionHash},
    inherent::Inherent,
    EquivocationLocator,
};

use super::history_store_index::HistoryStoreIndex;
use crate::{interface::HistoryInterface, HistoryTreeChunk};

pub enum HistoryStoreProxy {
    WithIndex(HistoryStoreIndex),
    WithoutIndex(Box<dyn HistoryInterface + Send + Sync>),
}

impl HistoryStoreProxy {
    pub fn supports_index(&self) -> bool {
        match self {
            HistoryStoreProxy::WithIndex(_) => true,
            HistoryStoreProxy::WithoutIndex(_) => false,
        }
    }

    pub fn history_index(&self) -> Option<&HistoryStoreIndex> {
        match self {
            HistoryStoreProxy::WithIndex(index) => Some(index),
            HistoryStoreProxy::WithoutIndex(_) => None,
        }
    }
}

impl HistoryInterface for HistoryStoreProxy {
    // Adds all the transactions included in a given block into the history store.
    fn add_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &Block,
        inherents: Vec<Inherent>,
    ) -> Option<(Blake2bHash, u64)> {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.add_block(txn, block, inherents),
            HistoryStoreProxy::WithoutIndex(store) => store.add_block(txn, block, inherents),
        }
    }

    /// Removes all transactions, from a given block number, from the history store.
    fn remove_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &MicroBlock,
        inherents: Vec<Inherent>,
    ) -> Option<u64> {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.remove_block(txn, block, inherents),
            HistoryStoreProxy::WithoutIndex(store) => store.remove_block(txn, block, inherents),
        }
    }

    /// Removes the full history associated with a given epoch.
    fn remove_history(&self, txn: &mut MdbxWriteTransaction, epoch_number: u32) -> Option<()> {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.remove_history(txn, epoch_number),
            HistoryStoreProxy::WithoutIndex(store) => store.remove_history(txn, epoch_number),
        }
    }

    /// Obtains the current history root at the given block.
    fn get_history_tree_root(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<Blake2bHash> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.get_history_tree_root(block_number, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.get_history_tree_root(block_number, txn_option)
            }
        }
    }

    /// Clears the history store.
    fn clear(&self, txn: &mut MdbxWriteTransaction) {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.clear(txn),
            HistoryStoreProxy::WithoutIndex(store) => store.clear(txn),
        }
    }

    /// Returns the length (i.e. the number of leaves) of the History Tree at a given block height.
    /// Note that this returns the number of leaves for only the epoch of the given block height,
    /// this is because we have separate History Trees for separate epochs.
    fn length_at(&self, block_number: u32, txn_option: Option<&MdbxReadTransaction>) -> u32 {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.length_at(block_number, txn_option),
            HistoryStoreProxy::WithoutIndex(store) => store.length_at(block_number, txn_option),
        }
    }

    /// Returns the total length of the History Tree at a given epoch number.
    /// The size of the history length is useful for getting a proof for a previous state
    /// of the history tree.
    fn total_len_at_epoch(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.total_len_at_epoch(epoch_number, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.total_len_at_epoch(epoch_number, txn_option)
            }
        }
    }

    /// Returns the first and last block number stored in the history store
    fn history_store_range(&self, txn_option: Option<&MdbxReadTransaction>) -> (u32, u32) {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.history_store_range(txn_option),
            HistoryStoreProxy::WithoutIndex(store) => store.history_store_range(txn_option),
        }
    }

    /// Same as `add_to_history_for_epoch` but calculates the `epoch_number` using
    /// `Policy` functions.
    fn add_to_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.add_to_history(txn, block_number, hist_txs)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.add_to_history(txn, block_number, hist_txs)
            }
        }
    }

    /// Adds a list of historic transactions to an existing history tree. It returns the root of the
    /// resulting tree and the total size of the transactions added.
    /// This function assumes that:
    ///     1. The transactions are pushed in increasing block number order.
    ///     2. All the blocks are consecutive.
    ///     3. We only push transactions for one epoch at a time.
    fn add_to_history_for_epoch(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.add_to_history_for_epoch(txn, epoch_number, block_number, hist_txs)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.add_to_history_for_epoch(txn, epoch_number, block_number, hist_txs)
            }
        }
    }

    /// Removes a number of historic transactions from an existing history tree. It returns the root
    /// of the resulting tree and the total size of of the transactions removed.
    fn remove_partial_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        num_hist_txs: usize,
    ) -> Option<(Blake2bHash, u64)> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.remove_partial_history(txn, epoch_number, num_hist_txs)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.remove_partial_history(txn, epoch_number, num_hist_txs)
            }
        }
    }

    fn tx_in_validity_window(
        &self,
        raw_tx_hash: &RawTransactionHash,
        txn_opt: Option<&MdbxReadTransaction>,
    ) -> bool {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.tx_in_validity_window(raw_tx_hash, txn_opt)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.tx_in_validity_window(raw_tx_hash, txn_opt)
            }
        }
    }

    /// Gets all historic transactions for a given block number.
    /// This method returns the transactions in the same order that they appear in the block.
    fn get_block_transactions(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.get_block_transactions(block_number, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.get_block_transactions(block_number, txn_option)
            }
        }
    }

    /// Gets all historic transactions for a given epoch.
    fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.get_epoch_transactions(epoch_number, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.get_epoch_transactions(epoch_number, txn_option)
            }
        }
    }

    /// Returns the number of historic transactions for a given epoch.
    fn num_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.num_epoch_transactions(epoch_number, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.num_epoch_transactions(epoch_number, txn_option)
            }
        }
    }

    /// Returns the number of historic transactions within the given block's epoch that occurred
    /// before the given block (inclusive).
    fn num_epoch_transactions_before(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.num_epoch_transactions_before(block_number, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.num_epoch_transactions_before(block_number, txn_option)
            }
        }
    }

    /// Returns all historic transactions within the given block's epoch that occurred after the
    /// given block (exclusive).
    fn get_epoch_transactions_after(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.get_epoch_transactions_after(block_number, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.get_epoch_transactions_after(block_number, txn_option)
            }
        }
    }

    /// Returns the `chunk_index`th chunk of size `chunk_size` for a given epoch.
    /// The return value consists of a vector of all the historic transactions in that chunk
    /// and a proof for these in the MMR.
    /// The `verifier_block_number` is the block the chunk proof should be verified against.
    /// That means that no leaf beyond this block is returned and that the proof should be
    /// verified with the history root from this block.
    fn prove_chunk(
        &self,
        epoch_number: u32,
        verifier_block_number: u32,
        chunk_size: usize,
        chunk_index: usize,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoryTreeChunk> {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.prove_chunk(
                epoch_number,
                verifier_block_number,
                chunk_size,
                chunk_index,
                txn_option,
            ),
            HistoryStoreProxy::WithoutIndex(store) => store.prove_chunk(
                epoch_number,
                verifier_block_number,
                chunk_size,
                chunk_index,
                txn_option,
            ),
        }
    }

    /// Creates a new history tree from chunks and returns the root hash.
    fn tree_from_chunks(
        &self,
        epoch_number: u32,
        chunks: Vec<(Vec<HistoricTransaction>, RangeProof<Blake2bHash>)>,
        txn: &mut MdbxWriteTransaction,
    ) -> Result<Blake2bHash, MMRError> {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.tree_from_chunks(epoch_number, chunks, txn)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.tree_from_chunks(epoch_number, chunks, txn)
            }
        }
    }

    /// Returns the block number of the last leaf in the history store
    fn get_last_leaf_block_number(&self, txn_option: Option<&MdbxReadTransaction>) -> Option<u32> {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.get_last_leaf_block_number(txn_option),
            HistoryStoreProxy::WithoutIndex(store) => store.get_last_leaf_block_number(txn_option),
        }
    }

    /// Check whether an equivocation proof at a given equivocation locator has
    /// already been included.
    fn has_equivocation_proof(
        &self,
        locator: EquivocationLocator,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> bool {
        match self {
            HistoryStoreProxy::WithIndex(index) => {
                index.has_equivocation_proof(locator, txn_option)
            }
            HistoryStoreProxy::WithoutIndex(store) => {
                store.has_equivocation_proof(locator, txn_option)
            }
        }
    }

    /// Proves the number of leaves in the history store for the given block.
    fn prove_num_leaves(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<SizeProof<Blake2bHash, HistoricTransaction>, MMRError> {
        match self {
            HistoryStoreProxy::WithIndex(index) => index.prove_num_leaves(block_number, txn_option),
            HistoryStoreProxy::WithoutIndex(store) => {
                store.prove_num_leaves(block_number, txn_option)
            }
        }
    }
}
