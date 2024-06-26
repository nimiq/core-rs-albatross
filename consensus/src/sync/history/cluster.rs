use std::{
    collections::VecDeque,
    fmt::Formatter,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use futures::{FutureExt, Stream, StreamExt};
use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain::{interface::HistoryInterface, Blockchain, HistoryTreeChunk, CHUNK_SIZE};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::Network, request::RequestError};
use nimiq_primitives::{networks::NetworkId, policy::Policy, slots_allocation::Validators};
use nimiq_transaction::historic_transaction::HistoricTransaction;
use parking_lot::RwLock;
use thiserror::Error;

use crate::{
    messages::{
        BatchSetError, BatchSetInfo, HistoryChunkError, RequestBatchSet, RequestHistoryChunk,
    },
    sync::{peer_list::PeerList, sync_queue::SyncQueue},
};

/// Error enumeration for history sync request
#[derive(Clone, Debug, Error)]
pub enum HistoryRequestError {
    /// Outbound request error
    #[error("Outbound error: {0}")]
    RequestError(#[from] RequestError),
    /// Remote batch set error
    #[error("Remote batch set error: {0}")]
    RemoteBatchSetError(#[from] BatchSetError),
    /// Remote batch set error
    #[error("Remote history chunk error: {0}")]
    RemoteHistoryChunkError(#[from] HistoryChunkError),
    /// Batch set info obtained doesn't match the requested hash
    #[error("Batch set info mismatch")]
    BatchSetInfoMismatch,
    /// Batch set info obtained is invalid
    #[error("Invalid Batch Set Info")]
    InvalidBatchSetInfo,
    /// Macro block obtained is invalid
    #[error("Invalid Macro Block")]
    InvalidMacroBlock,
    /// History size proof is invalid
    #[error("Invalid Size Proof")]
    InvalidSizeProof,
    /// History chunk is invalid
    #[error("Invalid History Chunk")]
    InvalidHistoryChunk,
}

/// Structure to keep track of the history downloaded
struct PendingBatchSet {
    /// Macro block to verify the downloaded history
    macro_block: MacroBlock,
    /// Total size of the history to download
    history_len: u64,
    /// Batch set index within the epoch
    batch_set_index: usize,
    /// Downloaded history
    history: Vec<HistoricTransaction>,
}

impl PendingBatchSet {
    fn is_complete(&self) -> bool {
        self.history_len == self.history.len() as u64
    }

    fn epoch_number(&self) -> u32 {
        self.macro_block.epoch_number()
    }
}

pub struct BatchSet {
    pub block: MacroBlock,
    pub history: Vec<HistoricTransaction>,
    pub batch_index: usize,
}

impl std::fmt::Debug for BatchSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("BatchSet");
        dbg.field("epoch_number", &self.block.epoch_number());
        dbg.field("", &self.block.epoch_number());
        dbg.field("history_len", &self.history.len());
        dbg.field("batch_index", &self.batch_index);
        dbg.finish()
    }
}

impl From<PendingBatchSet> for BatchSet {
    fn from(batch_set: PendingBatchSet) -> Self {
        Self {
            block: batch_set.macro_block,
            history: batch_set.history,
            batch_index: batch_set.batch_set_index,
        }
    }
}

pub struct BatchSetVerifyState {
    network: NetworkId,
    predecessor: Block,
    validators: Validators,
}

#[derive(Clone, Debug)]
pub struct HistoryChunkRequest {
    epoch_number: u32,
    block_number: u32,
    chunk_index: u64,
    history_root: Blake2bHash,
}

impl HistoryChunkRequest {
    pub fn from_block(macro_block: &MacroBlock, chunk_index: u64) -> Self {
        Self {
            epoch_number: macro_block.epoch_number(),
            block_number: macro_block.block_number(),
            chunk_index,
            history_root: macro_block.header.history_root.clone(),
        }
    }
}

static SYNC_CLUSTER_ID: AtomicUsize = AtomicUsize::new(0);

pub struct SyncCluster<TNetwork: Network> {
    pub id: usize,
    pub epoch_ids: Vec<Blake2bHash>,
    pub first_epoch_number: usize,
    pub first_block_number: usize,

    // Both batch_set_queue and the history_queue share the same peers.
    pub(crate) batch_set_queue:
        SyncQueue<TNetwork, Blake2bHash, BatchSetInfo, HistoryRequestError, BatchSetVerifyState>,
    history_queue: SyncQueue<
        TNetwork,
        HistoryChunkRequest,
        (HistoryChunkRequest, HistoryTreeChunk),
        HistoryRequestError,
        (),
    >,

    pending_batch_sets: VecDeque<PendingBatchSet>,
    num_epochs_finished: usize,

    blockchain: Arc<RwLock<Blockchain>>,
    network: Arc<TNetwork>,
}

impl<TNetwork: Network + 'static> SyncCluster<TNetwork> {
    const NUM_PENDING_BATCH_SETS: usize = 5;
    const NUM_PENDING_CHUNKS: usize = 12;

    pub(crate) fn for_epoch(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TNetwork>,
        peers: PeerList<TNetwork>,
        epoch_ids: Vec<Blake2bHash>,
        first_epoch_number: usize,
    ) -> Self {
        Self::new(
            blockchain,
            network,
            Arc::new(RwLock::new(peers)),
            epoch_ids,
            first_epoch_number,
            (first_epoch_number * Policy::blocks_per_epoch() as usize)
                + Policy::genesis_block_number() as usize,
        )
    }

    pub(crate) fn for_checkpoint(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TNetwork>,
        peers: PeerList<TNetwork>,
        checkpoint_id: Blake2bHash,
        epoch_number: usize,
        block_number: usize,
    ) -> Self {
        Self::new(
            blockchain,
            network,
            Arc::new(RwLock::new(peers)),
            vec![checkpoint_id],
            epoch_number,
            block_number,
        )
    }

    fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TNetwork>,
        peers: Arc<RwLock<PeerList<TNetwork>>>,
        epoch_ids: Vec<Blake2bHash>,
        first_epoch_number: usize,
        first_block_number: usize,
    ) -> Self {
        let id = SYNC_CLUSTER_ID.fetch_add(1, Ordering::SeqCst);

        let batch_verify_state = {
            let blockchain = blockchain.read();
            BatchSetVerifyState {
                network: blockchain.network_id,
                predecessor: Block::Macro(blockchain.election_head()),
                validators: blockchain.election_head().get_validators().unwrap(),
            }
        };
        let epoch_ids_queue = epoch_ids
            .iter()
            .map(|epoch_id| (epoch_id.clone(), None))
            .collect();
        let batch_set_queue = SyncQueue::with_verification(
            Arc::clone(&network),
            epoch_ids_queue,
            peers.clone(),
            Self::NUM_PENDING_BATCH_SETS,
            |id, network, peer_id| {
                async move { Self::request_epoch(network, peer_id, id).await }.boxed()
            },
            |_, batch_set_info, verify_state| {
                if let Err(e) = Self::verify_batch_set_info(
                    verify_state.network,
                    batch_set_info,
                    &verify_state.predecessor,
                    &verify_state.validators,
                ) {
                    warn!(error = ?e, "Received invalid batch set");
                    return false;
                }

                // Update verify state.
                let macro_block = batch_set_info.final_macro_block();
                verify_state.predecessor = Block::Macro(macro_block.clone());

                if macro_block.is_election() {
                    match macro_block.get_validators() {
                        Some(validators) => verify_state.validators = validators,
                        None => {
                            warn!("Received election block without validators");
                            return false;
                        }
                    }
                }

                true
            },
            batch_verify_state,
        );

        let history_queue = SyncQueue::new(
            Arc::clone(&network),
            Vec::<(HistoryChunkRequest, Option<_>)>::new(),
            peers,
            Self::NUM_PENDING_CHUNKS,
            move |request, network, peer_id| {
                async move {
                    Self::request_history_chunk(network, peer_id, request.clone())
                        .await
                        .map(|chunk| (request, chunk))
                }
                .boxed()
            },
        );
        Self {
            id,
            epoch_ids,
            first_epoch_number,
            first_block_number,
            batch_set_queue,
            history_queue,
            pending_batch_sets: VecDeque::with_capacity(Self::NUM_PENDING_BATCH_SETS),
            num_epochs_finished: 0,
            blockchain,
            network,
        }
    }

    fn verify_macro_block(
        block: &Block,
        network: NetworkId,
        macro_predecessor: &Block,
        validators: &Validators,
    ) -> Result<(), HistoryRequestError> {
        if let Err(error) = block.verify(network) {
            warn!(%block, %error, reason = "Block intrinsic checks failed", "Invalid macro block");
            return Err(HistoryRequestError::InvalidMacroBlock);
        }

        if let Err(error) = block.verify_macro_successor(macro_predecessor.unwrap_macro_ref()) {
            warn!(%block, %error, reason = "Block predecessor checks failed", "Invalid macro block");
            return Err(HistoryRequestError::InvalidMacroBlock);
        }

        if let Err(error) = block.verify_validators(validators) {
            warn!(%block, %error, reason = "Block verification for slot failed", "Invalid macro block");
            return Err(HistoryRequestError::InvalidMacroBlock);
        }

        Ok(())
    }

    fn verify_batch_set_info(
        network: NetworkId,
        batch_set_info: &BatchSetInfo,
        predecessor_macro_block: &Block,
        validators: &Validators,
    ) -> Result<(), HistoryRequestError> {
        // Check and verify the election macro block if it exists and get the parent election_hash of this epoch
        if let Some(election_macro_block) = &batch_set_info.election_macro_block {
            let block = Block::Macro(election_macro_block.clone());
            if !block.is_election() {
                warn!(%block, reason = "Invalid election block", "Block is not an election block");
                return Err(HistoryRequestError::InvalidBatchSetInfo);
            }

            Self::verify_macro_block(&block, network, predecessor_macro_block, validators)?;
        }

        let mut last_seen_macro_block = predecessor_macro_block.clone();

        // Now do some basic consistency checks for batch sets and verify the respective
        // macro block and size proof.
        for batch_set in &batch_set_info.batch_sets {
            let block = Block::Macro(batch_set.macro_block.clone());

            // Check that the received blocks within the batch sets are in order.
            if block.block_number() <= last_seen_macro_block.block_number() {
                warn!(%block, reason = "Decreasing block number", "Block has a decreasing block number");
                return Err(HistoryRequestError::BatchSetInfoMismatch);
            }

            // Check the macro block of the batch set.
            Self::verify_macro_block(&block, network, &last_seen_macro_block, validators)?;

            // Check the history size proof.
            if !batch_set.history_len.verify(block.history_root()) {
                return Err(HistoryRequestError::InvalidSizeProof);
            }

            last_seen_macro_block = block;
        }

        Ok(())
    }

    fn on_epoch_received(&mut self, epoch: BatchSetInfo) -> Result<(), SyncClusterResult> {
        let block = epoch.final_macro_block();
        let epoch_number = block.epoch_number();

        let blockchain = self.blockchain.read();
        let current_block_number = blockchain.block_number();
        if block.block_number() <= current_block_number {
            debug!("Received outdated epoch at block {}", current_block_number);
            return Err(SyncClusterResult::Outdated);
        }

        let current_epoch_number = blockchain.epoch_number();
        let num_known_txs = if epoch_number == current_epoch_number {
            blockchain
                .history_store
                .get_number_final_epoch_transactions(epoch_number, None) as u64
        } else {
            0
        };

        // Release the blockchain lock
        drop(blockchain);

        info!(
            "Syncing epoch #{}/{} ({} checkpoints, {} total history items)",
            epoch_number,
            self.first_epoch_number + self.len() - 1,
            epoch.batch_sets.len(),
            epoch.total_history_len(),
        );

        let mut epoch_processed_history_items = 0u64;

        for (index, batch_set) in epoch.batch_sets.iter().enumerate() {
            // If the batch_set is in the current epoch, skip already known history.
            // We can only skip if the batch_set is not empty, otherwise we can't tell by the
            // history size if it was already adopted or not.
            let epoch_history_offset =
                epoch_processed_history_items / CHUNK_SIZE as u64 * CHUNK_SIZE as u64;
            let cum_history_len = batch_set.history_len.size();
            if batch_set.history_len.size() > 0 && num_known_txs >= cum_history_len {
                // This chunk is already known to the blockchain
                epoch_processed_history_items = batch_set.history_len.size();
                continue;
            }

            // Compute the index of the first transaction of this batch set to download.
            let start_txn = if num_known_txs > epoch_history_offset {
                num_known_txs / CHUNK_SIZE as u64 * CHUNK_SIZE as u64
            } else {
                epoch_history_offset
            };

            // Now compute how many history items we need to download
            let history_len = batch_set.history_len.size() - start_txn;

            // Prepare pending info.
            let pending_batch_set = PendingBatchSet {
                macro_block: batch_set.macro_block.clone(),
                history_len,
                batch_set_index: index,
                history: Vec::new(),
            };

            log::debug!(
                epoch = %batch_set.macro_block.epoch_number(),
                block = %batch_set.macro_block,
                history_len,
                batch_set_index = index,
                "Adding pending batch set",
            );

            // Queue history chunks for the given batch set for download.
            let history_chunk_ids: Vec<(HistoryChunkRequest, Option<_>)> = (start_txn
                / CHUNK_SIZE as u64
                ..((batch_set.history_len.size()).div_ceil(CHUNK_SIZE as u64)))
                .map(|i| {
                    (
                        HistoryChunkRequest::from_block(&batch_set.macro_block, i),
                        None,
                    )
                })
                .collect();
            self.history_queue.add_ids(history_chunk_ids);

            // We keep the epoch in pending_epochs while the history is downloading.
            self.pending_batch_sets.push_back(pending_batch_set);

            // Set the total history that we have processed for this epoch
            epoch_processed_history_items = batch_set.history_len.size();
        }

        Ok(())
    }

    fn on_history_chunk_received(
        &mut self,
        epoch_number: u32,
        block_number: u32,
        mut history_chunk: HistoryTreeChunk,
    ) -> Result<(), SyncClusterResult> {
        // Find batch set in pending_batch_sets.
        let batch_set_idx = &mut self
            .pending_batch_sets
            .iter()
            .position(|batch_set| block_number == batch_set.macro_block.block_number())
            .ok_or_else(|| {
                log::error!(
                    "Batch set couldn't be found. Epoch {}, block {}",
                    epoch_number,
                    block_number,
                );
                SyncClusterResult::NoSuchBatchSet
            })?;

        let batch_set = &mut self.pending_batch_sets[*batch_set_idx];

        // Add the received history chunk to the pending epoch.
        batch_set.history.append(&mut history_chunk.history);

        log::info!(
            "Downloading history for epoch #{}, batch set #{}: {}/{} ({:.2}%)",
            batch_set.epoch_number(),
            batch_set.batch_set_index,
            batch_set.history.len(),
            batch_set.history_len,
            (batch_set.history.len() as f64 / batch_set.history_len as f64) * 100f64,
        );

        Ok(())
    }

    /// Adds the peer to both queues (history and batch set).
    pub(crate) fn add_peer(&mut self, peer_id: TNetwork::PeerId) -> bool {
        self.batch_set_queue.add_peer(peer_id)
    }

    /// Removes the peer from both queues (history and batch set).
    pub(crate) fn remove_peer(&mut self, peer_id: &TNetwork::PeerId) {
        self.batch_set_queue.remove_peer(peer_id);
    }

    /// Returns the shared list of peers of both queues (history and batch set).
    pub(crate) fn peers(&self) -> Vec<<TNetwork as Network>::PeerId> {
        self.batch_set_queue.peers.read().peers().to_vec()
    }

    pub(crate) fn split_off(&mut self, at: usize) -> Self {
        assert!(
            self.num_epochs_finished() <= at,
            "Cannot split cluster #{} at {}, already {} ids processed",
            self.id,
            at,
            self.num_epochs_finished()
        );

        let ids = self.epoch_ids.split_off(at);
        let first_epoch_number = self.first_epoch_number + at;

        // Remove the split-off ids from our epoch queue.
        self.batch_set_queue.truncate_ids(at);

        Self::for_epoch(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.network),
            self.batch_set_queue.peers.read().clone(), // makes sure we have a hard copy
            ids,
            first_epoch_number,
        )
    }

    pub(crate) fn remove_front(&mut self, num_items: usize) {
        // TODO Refactor
        *self = self.split_off(usize::min(num_items, self.len()));
    }

    pub(crate) fn compare(&self, other: &Self, current_epoch: usize) -> std::cmp::Ordering {
        let this_epoch_number = self.first_epoch_number.max(current_epoch);
        let other_epoch_number = other.first_epoch_number.max(current_epoch);

        let this_ids_len = self
            .epoch_ids
            .len()
            .saturating_sub(current_epoch.saturating_sub(self.first_epoch_number));
        let other_ids_len = other
            .epoch_ids
            .len()
            .saturating_sub(current_epoch.saturating_sub(other.first_epoch_number));

        this_epoch_number
            .cmp(&other_epoch_number) // Lower epoch first
            .then_with(|| {
                self.first_block_number
                    .cmp(&other.first_block_number)
                    .reverse()
            }) // Higher block number first (used for checkpoints only)
            .then_with(|| {
                self.batch_set_queue
                    .num_peers()
                    .cmp(&other.batch_set_queue.num_peers())
                    .reverse()
            }) // Higher peer count first
            .then_with(|| this_ids_len.cmp(&other_ids_len).reverse()) // More ids first
            .then_with(|| self.epoch_ids.cmp(&other.epoch_ids))
    }

    pub(crate) fn len(&self) -> usize {
        self.epoch_ids.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.epoch_ids.is_empty()
    }

    pub(crate) fn num_epochs_finished(&self) -> usize {
        self.num_epochs_finished
    }

    pub(crate) fn reset_verify_state(&mut self) {
        let blockchain = self.blockchain.read();
        let verify_state = BatchSetVerifyState {
            network: blockchain.network_id,
            predecessor: Block::Macro(blockchain.election_head()),
            validators: blockchain.election_head().get_validators().unwrap(),
        };
        self.batch_set_queue.set_verify_state(verify_state);
    }

    pub async fn request_epoch(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        hash: Blake2bHash,
    ) -> Result<BatchSetInfo, HistoryRequestError> {
        let batch_set_info = network
            .request(RequestBatchSet { hash: hash.clone() }, peer_id)
            .await??;

        // Check that BatchSetInfo is not empty.
        if batch_set_info.election_macro_block.is_none() && batch_set_info.batch_sets.is_empty() {
            return Err(HistoryRequestError::InvalidBatchSetInfo);
        }

        // Check that the received batch set info matches the requested hash.
        let block_hash = batch_set_info.final_macro_block().hash();
        if hash != block_hash {
            warn!(expected = %hash, received = %block_hash, "Received unexpected batch set");
            return Err(HistoryRequestError::BatchSetInfoMismatch);
        }

        Ok(batch_set_info)
    }

    pub async fn request_history_chunk(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        request: HistoryChunkRequest,
    ) -> Result<HistoryTreeChunk, HistoryRequestError> {
        let req = RequestHistoryChunk {
            epoch_number: request.epoch_number,
            block_number: request.block_number,
            chunk_index: request.chunk_index,
        };
        let chunk = network.request(req, peer_id).await??.chunk;

        // Verify that the chunk is valid.
        let leaf_index = request.chunk_index as usize * CHUNK_SIZE;
        if !chunk
            .verify(&request.history_root, leaf_index)
            .unwrap_or(false)
        {
            log::warn!(
                epoch_number = request.epoch_number,
                block_number = request.block_number,
                chunk_index = request.chunk_index,
                peer = %peer_id,
                "HistoryChunk failed to verify",
            );
            return Err(HistoryRequestError::InvalidHistoryChunk);
        }

        Ok(chunk)
    }

    fn pop_complete_epoch(&mut self) -> Option<PendingBatchSet> {
        if !self.pending_batch_sets.is_empty() && self.pending_batch_sets[0].is_complete() {
            self.num_epochs_finished += 1;
            self.pending_batch_sets.pop_front()
        } else {
            None
        }
    }
}

impl<TNetwork: Network + 'static> Stream for SyncCluster<TNetwork> {
    type Item = Result<BatchSet, SyncClusterResult>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while self.pending_batch_sets.len() < Self::NUM_PENDING_BATCH_SETS {
            let result = match self.batch_set_queue.poll_next_unpin(cx) {
                Poll::Ready(Some(result)) => result,
                _ => break,
            };

            match result {
                Ok(epoch) => {
                    if let Err(e) = self.on_epoch_received(epoch) {
                        return Poll::Ready(Some(Err(e)));
                    }

                    // Immediately emit the next epoch if it is already complete. This can only
                    // happen for empty epochs. Currently, only the first epoch can be empty
                    // as there are no rewards distributed in that epoch. Therefore, it is
                    // sufficient to check for this condition here as opposed to in every call
                    // to poll_next().
                    if let Some(batch_set) = self.pop_complete_epoch() {
                        return Poll::Ready(Some(Ok(batch_set.into())));
                    }
                }
                Err(e) => {
                    log::debug!(
                        "Polling the batch set queue encountered error result: {:?}",
                        e
                    );
                    // TODO Improve error
                    return Poll::Ready(Some(Err(SyncClusterResult::Error)));
                }
            }
        }

        while let Poll::Ready(Some(result)) = self.history_queue.poll_next_unpin(cx) {
            match result {
                Ok((request, history_chunk)) => {
                    if let Err(e) = self.on_history_chunk_received(
                        request.epoch_number,
                        request.block_number,
                        history_chunk,
                    ) {
                        return Poll::Ready(Some(Err(e)));
                    }

                    // Emit finished epochs.
                    if let Some(batch_set) = self.pop_complete_epoch() {
                        return Poll::Ready(Some(Ok(batch_set.into())));
                    }
                }
                Err(e) => {
                    log::debug!(
                        epoch_number = e.epoch_number,
                        block_number = e.block_number,
                        chunk_index = e.chunk_index,
                        "Polling the history queue resulted in an error"
                    );
                    return Poll::Ready(Some(Err(SyncClusterResult::Error)));
                } // TODO Error
            }
        }

        // We're done if there are no more epochs to process.
        if self.batch_set_queue.is_empty() && self.pending_batch_sets.is_empty() {
            return Poll::Ready(None);
        }

        Poll::Pending
    }
}

impl<TNetwork: Network + 'static> std::fmt::Debug for SyncCluster<TNetwork> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("SyncCluster");
        dbg.field("id", &self.id);
        if Policy::is_election_block_at(self.first_block_number as u32) {
            // Epoch cluster
            let last_epoch_number =
                self.first_epoch_number + self.epoch_ids.len().saturating_sub(1);
            dbg.field("first_epoch_number", &self.first_epoch_number);
            dbg.field("last_epoch_number", &last_epoch_number);
            dbg.field("num_epoch_ids", &self.epoch_ids.len());
        } else {
            // Checkpoint cluster
            dbg.field("epoch_number", &self.first_epoch_number);
            dbg.field("block_number", &self.first_block_number);
        }
        dbg.field("num_peers", &self.peers().len());
        dbg.field("num_pending_batch_sets", &self.pending_batch_sets.len());
        dbg.field("num_epochs_finished", &self.num_epochs_finished);
        dbg.finish()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SyncClusterResult {
    EpochSuccessful,
    NoMoreEpochs,
    NoSuchBatchSet,
    InvalidBatchSet,
    InconsistentState,
    Error,
    Outdated,
}

impl From<Result<PushResult, PushError>> for SyncClusterResult {
    fn from(result: Result<PushResult, PushError>) -> Self {
        match result {
            Ok(PushResult::Extended | PushResult::Rebranched) => SyncClusterResult::EpochSuccessful,
            Ok(_) => SyncClusterResult::Outdated,
            Err(_) => SyncClusterResult::Error,
        }
    }
}
