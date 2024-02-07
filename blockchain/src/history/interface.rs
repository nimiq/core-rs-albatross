use nimiq_block::Block;
use nimiq_database::{TransactionProxy, WriteTransactionProxy};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_mmr::{
    error::Error as MMRError,
    mmr::proof::{RangeProof, SizeProof},
};
use nimiq_transaction::{
    historic_transaction::HistoricTransaction, history_proof::HistoryTreeProof, EquivocationLocator,
};

use crate::HistoryTreeChunk;

/// Defines several methods to interact with a history store.
pub trait HistoryInterface {
    /// Adds all the transactions included in a given block into the history store.
    fn add_block(&self, txn: &mut WriteTransactionProxy, block: &Block) -> Option<Blake2bHash>;

    /// Removes all transactions, from a given block number, from the history store.
    fn remove_block(&self, txn: &mut WriteTransactionProxy, block_number: u32) -> u64;

    /// Removes the full history associated with a given epoch.
    fn remove_history(&self, txn: &mut WriteTransactionProxy, epoch_number: u32) -> Option<()>;

    /// Obtains the current history root at the given epoch.
    fn get_history_tree_root(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<Blake2bHash>;

    /// Clears the history store.
    fn clear(&self, txn: &mut WriteTransactionProxy);

    /// Returns the length (i.e. the number of leaves) of the History Tree at a given block height.
    /// Note that this returns the number of leaves for only the epoch of the given block height,
    /// this is because we have separate History Trees for separate epochs.
    fn length_at(&self, block_number: u32, txn_option: Option<&TransactionProxy>) -> u32;

    /// Returns the total length of the History Tree at a given epoch number.
    /// The size of the history length is useful for getting a proof for a previous state
    /// of the history tree.
    fn total_len_at_epoch(&self, epoch_number: u32, txn_option: Option<&TransactionProxy>)
        -> usize;

    /// Add a list of historic transactions to an existing history tree. It returns the root of the
    /// resulting tree and the total size of the transactions added.
    /// This function assumes that:
    ///     1. The transactions are pushed in increasing block number order.
    ///     2. All the blocks are consecutive.
    ///     3. We only push transactions for one epoch at a time.
    fn add_to_history(
        &self,
        txn: &mut WriteTransactionProxy,
        epoch_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)>;

    /// Removes a number of historic transactions from an existing history tree. It returns the root
    /// of the resulting tree and the total size of of the transactions removed.
    fn remove_partial_history(
        &self,
        txn: &mut WriteTransactionProxy,
        epoch_number: u32,
        num_hist_txs: usize,
    ) -> Option<(Blake2bHash, u64)>;

    /// Calculates the history tree root from a vector of historic transactions. It doesn't use the
    /// database, it is just used to check the correctness of the history root when syncing.
    fn root_from_hist_txs(hist_txs: &[HistoricTransaction]) -> Option<Blake2bHash>;

    /// Gets an historic transaction given its transaction hash.
    fn get_hist_tx_by_hash(
        &self,
        tx_hash: &Blake2bHash,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction>;

    /// Gets all historic transactions for a given block number.
    /// This method returns the transactions in the same order that they appear in the block.
    fn get_block_transactions(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction>;

    /// Gets all historic transactions for a given epoch.
    fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction>;

    /// Returns the number of historic transactions for a given epoch.
    fn num_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> usize;

    /// Gets all finalized historic transactions for a given epoch.
    fn get_final_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction>;

    /// Gets the number of all finalized historic transactions for a given epoch.
    /// This is basically an optimization of calling `get_final_epoch_transactions(..).len()`
    /// since the latter is very expensive
    fn get_number_final_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> usize;

    /// Gets all non-finalized historic transactions for a given epoch.
    fn get_nonfinal_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<HistoricTransaction>;

    /// Returns a vector containing all transaction (and reward inherents) hashes corresponding to the given
    /// address. It fetches the transactions from most recent to least recent up to the maximum
    /// number given.
    fn get_tx_hashes_by_address(
        &self,
        address: &Address,
        max: u16,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<Blake2bHash>;

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
    ) -> Option<HistoryTreeProof>;

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
        txn_option: Option<&TransactionProxy>,
    ) -> Option<HistoryTreeChunk>;

    /// Creates a new history tree from chunks and returns the root hash.
    fn tree_from_chunks(
        &self,
        epoch_number: u32,
        chunks: Vec<(Vec<HistoricTransaction>, RangeProof<Blake2bHash>)>,
        txn: &mut WriteTransactionProxy,
    ) -> Result<Blake2bHash, MMRError>;

    /// Returns the block number of the last leaf in the history store
    fn get_last_leaf_block_number(&self, txn_option: Option<&TransactionProxy>) -> Option<u32>;

    /// Check whether an equivocation proof at a given equivocation locator has
    /// already been included.
    fn has_equivocation_proof(
        &self,
        locator: EquivocationLocator,
        txn_option: Option<&TransactionProxy>,
    ) -> bool;

    /// Proves the number of leaves in the history store for the given block.
    fn prove_num_leaves(
        &self,
        block_number: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<SizeProof<Blake2bHash, HistoricTransaction>, MMRError>;
}
