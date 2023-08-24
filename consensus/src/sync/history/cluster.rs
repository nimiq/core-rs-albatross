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
use nimiq_blockchain::{Blockchain, HistoryTreeChunk, CHUNK_SIZE};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::Network, request::RequestError};
use nimiq_primitives::{policy::Policy, slots_allocation::Validators};
use nimiq_transaction::extended_transaction::ExtendedTransaction;
use nimiq_utils::math::CeilingDiv;
use parking_lot::RwLock;
use thiserror::Error;

use crate::{
    messages::{BatchSetInfo, HistoryChunk, RequestBatchSet, RequestHistoryChunk},
    sync::{peer_list::PeerList, sync_queue::SyncQueue},
};

/// Error enumeration for history sync request
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HistoryRequestError {
    /// Outbound request error
    #[error("Outbound error: {0}")]
    RequestError(RequestError),
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
}

struct PendingBatchSet {
    macro_block: MacroBlock,
    history_len: usize,
    history_offset: usize,
    batch_index: usize,
    history: Vec<ExtendedTransaction>,
}
impl PendingBatchSet {
    fn is_complete(&self) -> bool {
        self.history_len == self.history.len() + self.history_offset
    }

    fn epoch_number(&self) -> u32 {
        self.macro_block.epoch_number()
    }
}

pub struct BatchSet {
    pub block: MacroBlock,
    pub history: Vec<ExtendedTransaction>,
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
            batch_index: batch_set.batch_index,
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
    pub(crate) batch_set_queue: SyncQueue<TNetwork, Blake2bHash, BatchSetInfo>,
    history_queue: SyncQueue<TNetwork, (u32, u32, usize), (u32, u32, usize, HistoryTreeChunk)>,

    pending_batch_sets: VecDeque<PendingBatchSet>,
    num_epochs_finished: usize,
    last_macro_block_emitted: Option<MacroBlock>,

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

        let batch_set_queue = SyncQueue::new(
            Arc::clone(&network),
            epoch_ids.clone(),
            peers.clone(),
            Self::NUM_PENDING_BATCH_SETS,
            |id, network, peer_id| {
                async move {
                    if let Ok(batch) = Self::request_epoch(network, peer_id, id).await {
                        // Two possible cases:
                        // 1. Got a batch set info for a complete epoch (that does have an election macro block)
                        // 2. Got a batch set info for a partial epoch (that doesn't have an election block yet)
                        if batch.election_macro_block.is_some() || !batch.batch_sets.is_empty() {
                            return Some(batch);
                        }
                    }
                    None
                }
                .boxed()
            },
        );
        let history_queue = SyncQueue::new(
            Arc::clone(&network),
            Vec::<(u32, u32, usize)>::new(),
            peers,
            Self::NUM_PENDING_CHUNKS,
            move |(epoch_number, block_number, chunk_index), network, peer_id| {
                async move {
                    Self::request_history_chunk(
                        network,
                        peer_id,
                        epoch_number,
                        block_number,
                        chunk_index,
                    )
                    .await
                    .ok()
                    .filter(|chunk| chunk.chunk.is_some())
                    .map(|chunk| {
                        (
                            epoch_number,
                            block_number,
                            chunk_index,
                            chunk.chunk.unwrap(),
                        )
                    })
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
            last_macro_block_emitted: None,
            blockchain,
            network,
        }
    }

    fn verify_macro_block(
        block: &Block,
        macro_predecessor: &Block,
        validators: &Validators,
    ) -> Result<(), HistoryRequestError> {
        if let Err(error) = block.verify() {
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
        batch_set_info: &BatchSetInfo,
        blockchain_head_block_number: u32,
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

            Self::verify_macro_block(&block, predecessor_macro_block, validators)?;
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

            // Don't verify blocks that are known from an epoch since they are
            // going to be dismissed later.
            if block.block_number() <= blockchain_head_block_number {
                continue;
            }

            // Check the macro block of the batch set.
            Self::verify_macro_block(&block, &last_seen_macro_block, validators)?;

            // Check the history size proof.
            if !batch_set.history_len.verify(block.history_root()) {
                return Err(HistoryRequestError::InvalidSizeProof);
            }

            last_seen_macro_block = block;
        }

        Ok(())
    }

    fn on_epoch_received(&mut self, epoch: BatchSetInfo) -> Result<(), SyncClusterResult> {
        // `epoch.block` is Some or epoch.batch_sets is not empty, since we filtered it accordingly
        // in the `request_fn`. Also the `request_fn` already verified we obtained a BatchSetInfo for
        // the block hash we are expecting
        let block = if let Some(ref election_macro_block) = epoch.election_macro_block {
            election_macro_block.clone()
        } else {
            epoch
                .batch_sets
                .last()
                .expect("Batch sets should not be empty")
                .macro_block
                .clone()
        };

        info!(
            "Syncing epoch #{}/{} ({} checkpoints, {} total history items)",
            block.epoch_number(),
            self.first_epoch_number + self.len() - 1,
            epoch.batch_sets.len(),
            epoch.total_history_len(),
        );

        let epoch_number = block.epoch_number();

        let blockchain = self.blockchain.read();
        let current_block_number = blockchain.block_number();
        let current_epoch_number = blockchain.epoch_number();
        let num_known_txs = if epoch_number == current_epoch_number {
            blockchain
                .history_store
                .get_number_final_epoch_transactions(epoch_number, None)
        } else {
            0
        };

        if block.header.block_number <= current_block_number {
            debug!("Received outdated epoch at block {}", current_block_number);
            return Err(SyncClusterResult::Outdated);
        }

        // The last known validators:
        // 1. The validators of the last epoch pending to download, which are only present if the
        //    epoch is complete (i.e. the corresponding macro block is an election block).
        // 2. The validators defined in the last macro block we returned from this cluster if it was
        //    an election block.
        // 3. The blockchain's current validators.
        let last_known_validators = self
            .pending_batch_sets
            .back()
            .and_then(|batch_set| batch_set.macro_block.get_validators())
            .or_else(|| {
                self.last_macro_block_emitted
                    .as_ref()
                    .and_then(|macro_block| macro_block.get_validators())
            })
            .or_else(|| blockchain.current_validators())
            .expect("Failed to determine last known validators");

        // The expected predecessor macro block:
        // 1. The macro block of the last epoch pending to download
        // 2. The last macro block we returned from this cluster (might not have been pushed onto the chain yet)
        // 3. The macro head of the blockchain
        let macro_head = blockchain.macro_head();
        let predecessor_macro = if let Some(last_pending_batch_set) = self.pending_batch_sets.back()
        {
            &last_pending_batch_set.macro_block
        } else if let Some(last_macro_block) = self.last_macro_block_emitted.as_ref() {
            last_macro_block
        } else {
            // The pending batch set is empty, so the predecessor comes from the blockchain
            &macro_head
        };

        // Release the blockchain lock
        drop(blockchain);

        // Verify the batch set info received
        Self::verify_batch_set_info(
            &epoch,
            current_block_number,
            &Block::Macro(predecessor_macro.clone()),
            &last_known_validators,
        )
        .map_err(|_| SyncClusterResult::InvalidBatchSet)?;

        let mut previous_history_size = 0usize;

        for (index, batch_set) in epoch.batch_sets.iter().enumerate() {
            // If the batch_set is in the current epoch, skip already known history.
            // We can only skip if the batch_set is not empty, otherwise we can't tell by the
            // history size if it was already adopted or not.
            let batch_set_epoch_boundary = previous_history_size / CHUNK_SIZE * CHUNK_SIZE
                + batch_set.history_len.size() as usize;
            if batch_set.history_len.size() > 0 && num_known_txs >= batch_set_epoch_boundary {
                // This chunk is already known to the blockchain
                previous_history_size = batch_set.history_len.size() as usize;
                continue;
            }

            // Compute the index of the first transaction to download.
            let start_txn = if current_epoch_number == epoch_number {
                num_known_txs.saturating_sub(previous_history_size / CHUNK_SIZE * CHUNK_SIZE)
            } else {
                0
            };

            // Prepare pending info.
            let pending_batch_set = PendingBatchSet {
                macro_block: batch_set.macro_block.clone(),
                history_len: batch_set.history_len.size() as usize - previous_history_size,
                history_offset: start_txn / CHUNK_SIZE * CHUNK_SIZE,
                batch_index: index,
                history: Vec::new(),
            };

            log::debug!(
                epoch = %batch_set.macro_block.epoch_number(),
                block = %batch_set.macro_block,
                history_len = batch_set.history_len.size(),
                batch_index = index,
                "Adding pending batch",
            );

            // Queue history chunks for the given batch set for download.
            let history_chunk_ids: Vec<(u32, u32, usize)> = (start_txn / CHUNK_SIZE
                ..((batch_set.history_len.size() as usize).ceiling_div(CHUNK_SIZE)))
                .map(|i| {
                    (
                        epoch_number,
                        pending_batch_set.macro_block.header.block_number,
                        previous_history_size / CHUNK_SIZE + i,
                    )
                })
                .collect();
            self.history_queue.add_ids(history_chunk_ids);

            // We keep the epoch in pending_epochs while the history is downloading.
            self.pending_batch_sets.push_back(pending_batch_set);

            // Set the previous history size.
            previous_history_size = batch_set.history_len.size() as usize;
        }

        Ok(())
    }

    fn on_history_chunk_received(
        &mut self,
        epoch_number: u32,
        block_number: u32,
        chunk_index: usize,
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

        // Verify chunk.
        if !history_chunk
            .verify(
                batch_set.macro_block.header.history_root.clone(),
                chunk_index * CHUNK_SIZE,
            )
            .unwrap_or(false)
        {
            log::warn!(
                "History Chunk failed to verify (chunk {} of epoch {})",
                chunk_index,
                epoch_number
            );
            return Err(SyncClusterResult::Error);
        }

        // Add the received history chunk to the pending epoch.
        batch_set.history.append(&mut history_chunk.history);

        if batch_set.history_len > CHUNK_SIZE {
            log::info!(
                "Downloading history for epoch #{}: {}/{} ({:.2}%)",
                batch_set.epoch_number(),
                batch_set.history.len(),
                batch_set.history_len,
                ((batch_set.history.len() + batch_set.history_offset) as f64
                    / batch_set.history_len as f64)
                    * 100f64,
            );
        }

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
        self.batch_set_queue.peers.read().peers().clone()
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

    pub async fn request_epoch(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        hash: Blake2bHash,
    ) -> Result<BatchSetInfo, HistoryRequestError> {
        let batch_set_info = network
            .request(RequestBatchSet { hash: hash.clone() }, peer_id)
            .await
            .map_err(HistoryRequestError::RequestError)?;

        // Check that the received batch set info matches the requested hash
        // Two possible cases:
        // 1. Got a batch set info for a complete epoch (that does have an election macro block)
        // 2. Got a batch set info for a partial epoch (that doesn't have an election block yet)
        let block_hash = if let Some(ref election_macro_block) = batch_set_info.election_macro_block
        {
            election_macro_block.hash()
        } else {
            // This is for an epoch that hasn't finalized. Now check and see if the batch set is properly built
            // and that the last batch set does have a macro block
            batch_set_info
                .batch_sets
                .last()
                .ok_or(HistoryRequestError::InvalidBatchSetInfo)?
                .macro_block
                .hash()
        };
        if hash != block_hash {
            warn!(expected = %hash, received = %block_hash, "Received unexpected block");
            return Err(HistoryRequestError::BatchSetInfoMismatch);
        }
        Ok(batch_set_info)
    }

    pub async fn request_history_chunk(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        epoch_number: u32,
        block_number: u32,
        chunk_index: usize,
    ) -> Result<HistoryChunk, RequestError> {
        network
            .request(
                RequestHistoryChunk {
                    epoch_number,
                    block_number,
                    chunk_index: chunk_index as u64,
                },
                peer_id,
            )
            .await
    }

    fn pop_complete_epoch(&mut self) -> Option<PendingBatchSet> {
        if !self.pending_batch_sets.is_empty() && self.pending_batch_sets[0].is_complete() {
            self.num_epochs_finished += 1;
            let epoch = self.pending_batch_sets.pop_front().unwrap();
            self.last_macro_block_emitted = Some(epoch.macro_block.clone());
            Some(epoch)
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
                Ok((epoch_number, block_number, chunk_index, history_chunk)) => {
                    if let Err(e) = self.on_history_chunk_received(
                        epoch_number,
                        block_number,
                        chunk_index,
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
                    log::debug!("Polling the history queue resulted in an error for epoch #{}, verifier_block_number: #{}, history_chunk: #{}", e.0, e.1, e.2);
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
