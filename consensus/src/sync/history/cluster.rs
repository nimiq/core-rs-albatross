use std::collections::VecDeque;
use std::fmt::Formatter;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};
use parking_lot::RwLock;

use nimiq_block::MacroBlock;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, ExtendedTransaction, CHUNK_SIZE};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::prelude::Peer;
use nimiq_utils::math::CeilingDiv;

use crate::consensus_agent::ConsensusAgent;
use crate::messages::{BatchSetInfo, HistoryChunk};
use crate::sync::sync_queue::{SyncQueue, SyncQueuePeer};

struct PendingBatchSet {
    block: MacroBlock,
    history_len: usize,
    history_offset: usize,
    history: Vec<ExtendedTransaction>,
}
impl PendingBatchSet {
    fn is_complete(&self) -> bool {
        self.history_len == self.history.len() + self.history_offset
    }

    fn epoch_number(&self) -> u32 {
        self.block.epoch_number()
    }
}

pub(crate) struct BatchSet {
    pub block: MacroBlock,
    pub history: Vec<ExtendedTransaction>,
}

impl std::fmt::Debug for BatchSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("BatchSet");
        dbg.field("epoch_number", &self.block.epoch_number());
        dbg.field("history_len", &self.history.len());
        dbg.finish()
    }
}

impl From<PendingBatchSet> for BatchSet {
    fn from(batch_set: PendingBatchSet) -> Self {
        Self {
            block: batch_set.block,
            history: batch_set.history,
        }
    }
}

static SYNC_CLUSTER_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct SyncCluster<TPeer: Peer> {
    pub id: usize,
    pub epoch_ids: Vec<Blake2bHash>,
    pub first_epoch_number: usize,

    pub(crate) batch_set_queue: SyncQueue<TPeer, Blake2bHash, BatchSetInfo>,
    history_queue: SyncQueue<TPeer, (u32, u32, usize), (u32, HistoryChunk)>,

    pending_batch_sets: VecDeque<PendingBatchSet>,
    num_epochs_finished: usize,

    blockchain: Arc<RwLock<Blockchain>>,
}

impl<TPeer: Peer + 'static> SyncCluster<TPeer> {
    const NUM_PENDING_BATCH_SETS: usize = 5;
    const NUM_PENDING_CHUNKS: usize = 12;

    pub(crate) fn new(
        epoch_ids: Vec<Blake2bHash>,
        first_epoch_number: usize,
        peers: Vec<SyncQueuePeer<TPeer>>,
        blockchain: Arc<RwLock<Blockchain>>,
    ) -> Self {
        let id = SYNC_CLUSTER_ID.fetch_add(1, Ordering::SeqCst);

        let batch_set_queue = SyncQueue::new(
            epoch_ids.clone(),
            peers.clone(),
            Self::NUM_PENDING_BATCH_SETS,
            |id, peer| {
                async move {
                    if let Some(peer) = Weak::upgrade(&peer) {
                        if let Ok(batch) = peer.request_epoch(id).await {
                            if batch.block.is_some() {
                                return Some(batch);
                            }
                        }
                    }
                    None
                }
                .boxed()
            },
        );
        let history_queue = SyncQueue::new(
            Vec::<(u32, u32, usize)>::new(),
            peers,
            Self::NUM_PENDING_CHUNKS,
            move |(epoch_number, block_number, chunk_index), peer| {
                async move {
                    if let Some(peer) = Weak::upgrade(&peer) {
                        return peer
                            .request_history_chunk(epoch_number, block_number, chunk_index)
                            .await
                            .ok()
                            .map(|chunk| (epoch_number, chunk));
                    }
                    None
                }
                .boxed()
            },
        );
        Self {
            id,
            epoch_ids,
            first_epoch_number,
            batch_set_queue,
            history_queue,
            pending_batch_sets: VecDeque::with_capacity(Self::NUM_PENDING_BATCH_SETS),
            num_epochs_finished: 0,
            blockchain,
        }
    }

    fn on_epoch_received(&mut self, epoch: BatchSetInfo) -> Result<(), SyncClusterResult> {
        // `epoch.block` is Some, since we filtered it accordingly in the `request_fn`
        let block = epoch.block.expect("epoch.block should exist");

        info!(
            "Syncing epoch #{}/{} ({} history items)",
            block.epoch_number(),
            self.first_epoch_number + self.len() - 1,
            epoch.history_len
        );

        // TODO Verify macro blocks and their ordering
        // Currently we only do a very basic check here
        let blockchain = self.blockchain.read();
        let current_block_number = blockchain.block_number();
        if block.header.block_number <= current_block_number {
            debug!("Received outdated epoch at block {}", current_block_number);
            return Err(SyncClusterResult::Outdated);
        }

        // Prepare pending info.
        let mut pending_batch_set = PendingBatchSet {
            block,
            history_len: epoch.history_len as usize,
            history_offset: 0,
            history: Vec::new(),
        };

        // If the block is in the same epoch, add already known history.
        let epoch_number = pending_batch_set.block.epoch_number();

        let mut start_index = 0;
        if blockchain.epoch_number() == epoch_number {
            let num_known_txs = blockchain
                .history_store
                .num_final_epoch_transactions(epoch_number, None);
            start_index = num_known_txs / CHUNK_SIZE;
            pending_batch_set.history_offset = start_index * CHUNK_SIZE;
        }

        // Queue history chunks for the given epoch for download.
        let history_chunk_ids = (start_index
            ..((epoch.history_len as usize).ceiling_div(CHUNK_SIZE)))
            .map(|i| (epoch_number, pending_batch_set.block.header.block_number, i))
            .collect();
        self.history_queue.add_ids(history_chunk_ids);

        // We keep the epoch in pending_epochs while the history is downloading.
        self.pending_batch_sets.push_back(pending_batch_set);

        Ok(())
    }

    fn on_history_chunk_received(
        &mut self,
        epoch_number: u32,
        history_chunk: HistoryChunk,
    ) -> Result<(), SyncClusterResult> {
        // Find epoch in pending_epochs.
        // TODO: This assumes that epochs are always dense in `pending_batch_sets`
        //  which might not be the case for misbehaving peers.
        let first_epoch_number = self.pending_batch_sets[0].epoch_number();
        let epoch_index = (epoch_number - first_epoch_number) as usize;
        let epoch = &mut self.pending_batch_sets[epoch_index];

        // TODO: This assumes that we have already filtered responses with no chunk.
        if history_chunk.chunk.is_none() {
            log::error!("Received empty history chunk {:?}", history_chunk);
            return Err(SyncClusterResult::Error);
        }

        // Verify chunk.
        let chunk = history_chunk.chunk.expect("History chunk missing");
        if !chunk
            .verify(epoch.block.header.history_root.clone(), epoch.history.len())
            .unwrap_or(false)
        {
            log::debug!("History Chunk failed to verify");
            return Err(SyncClusterResult::Error);
        }

        // Add the received history chunk to the pending epoch.
        let mut chunk = chunk.history;
        epoch.history.append(&mut chunk);

        if epoch.history_len > CHUNK_SIZE {
            log::info!(
                "Downloading history for epoch #{}: {}/{} ({:.0}%)",
                epoch.epoch_number(),
                epoch.history.len(),
                epoch.history_len,
                (epoch.history.len() as f64 / epoch.history_len as f64) * 100f64
            );
        }

        Ok(())
    }

    pub(crate) fn add_peer(
        &mut self,
        peer_id: TPeer::Id,
        peer: Weak<ConsensusAgent<TPeer>>,
    ) -> bool {
        // TODO keep only one list of peers
        if !self.batch_set_queue.has_peer(peer_id.clone()) {
            self.batch_set_queue
                .add_peer(peer_id.clone(), Weak::clone(&peer));
            self.history_queue.add_peer(peer_id, peer);

            return true;
        }
        false
    }

    pub(crate) fn remove_peer(&mut self, peer_id: &TPeer::Id) {
        self.batch_set_queue.remove_peer(peer_id);
        self.history_queue.remove_peer(peer_id);
    }

    pub(crate) fn has_peer(&self, peer_id: TPeer::Id) -> bool {
        self.batch_set_queue.has_peer(peer_id)
    }

    pub(crate) fn peers(&self) -> &Vec<SyncQueuePeer<TPeer>> {
        &self.batch_set_queue.peers
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

        Self::new(
            ids,
            first_epoch_number,
            self.batch_set_queue.peers.clone(),
            Arc::clone(&self.blockchain),
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
                other
                    .batch_set_queue
                    .num_peers()
                    .cmp(&self.batch_set_queue.num_peers())
            }) // Higher peer count first
            .then_with(|| other_ids_len.cmp(&this_ids_len)) // More ids first
            .then_with(|| self.epoch_ids.cmp(&other.epoch_ids)) //
            .reverse() // We want the best cluster to be *last*
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
}

impl<TPeer: Peer + 'static> Stream for SyncCluster<TPeer> {
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
                    if self.pending_batch_sets[0].is_complete() {
                        self.num_epochs_finished += 1;
                        let batch_set = self.pending_batch_sets.pop_front().unwrap();
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
                Ok((epoch_number, history_chunk)) => {
                    if let Err(e) = self.on_history_chunk_received(epoch_number, history_chunk) {
                        return Poll::Ready(Some(Err(e)));
                    }

                    // Emit finished epochs.
                    if self.pending_batch_sets[0].is_complete() {
                        self.num_epochs_finished += 1;
                        let batch_set = self.pending_batch_sets.pop_front().unwrap();
                        return Poll::Ready(Some(Ok(batch_set.into())));
                    }
                }
                Err(e) => {
                    log::debug!("Polling the history queue resulted in an error for epoch #{}, verifier_block_number : #{}, history_chunk: #{}", e.0, e.1, e.2);
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

impl<TPeer: Peer + 'static> std::fmt::Debug for SyncCluster<TPeer> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("SyncCluster");
        dbg.field("id", &self.id);
        dbg.field("first_epoch_number", &self.first_epoch_number);
        dbg.field("num_epoch_ids", &self.epoch_ids.len());
        dbg.field("num_peers", &self.peers().len());
        dbg.field("num_pending_batch_sets", &self.pending_batch_sets.len());
        dbg.field("num_epochs_finished", &self.num_epochs_finished);
        dbg.finish()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SyncClusterResult {
    EpochSuccessful,
    NoMoreEpochs,
    Error,
    Outdated,
}

impl<T, E: std::fmt::Debug> From<Result<T, E>> for SyncClusterResult {
    fn from(res: Result<T, E>) -> Self {
        match res {
            Ok(_) => SyncClusterResult::EpochSuccessful,
            Err(err) => {
                log::debug!(
                    "SyncClusterResult From<Result<T, E>> encountered error: {:?}",
                    err
                );
                SyncClusterResult::Error
            }
        }
    }
}
