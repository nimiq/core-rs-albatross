use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use futures::{future::BoxFuture, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_network_interface::network::Network;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

#[cfg(feature = "full")]
use self::state_queue::StateQueue;
use self::{block_queue::BlockQueue, queue::LiveSyncQueue};
use super::sync_interface::{LiveSync, LiveSyncEvent};
use crate::{
    consensus::ResolveBlockRequest,
    sync::live::block_queue::{BlockAndSource, BlockSource},
    BlsCache,
};

pub mod block_queue;
#[cfg(feature = "full")]
pub mod diff_queue;
pub mod queue;
#[cfg(feature = "full")]
pub mod state_queue;

/// Live sync stream using `BlockQueue`.
pub type BlockLiveSync<N> = LiveSyncer<N, BlockQueue<N>>;
/// Live sync stream using `StateQueue`.
#[cfg(feature = "full")]
pub type StateLiveSync<N> = LiveSyncer<N, StateQueue<N>>;

/// The maximum capacity of the external block stream passed into the block queue.
const MAX_BLOCK_STREAM_BUFFER: usize = 256;

/// The `LiveSyncer` manages the live sync process by coordinating block and chunk
/// processing through a `LiveSyncQueue`.
pub struct LiveSyncer<N: Network, Q: LiveSyncQueue<N>> {
    blockchain: BlockchainProxy,

    network: Arc<N>,

    queue: Q,

    /// Vector of pending push operations.
    pending: VecDeque<BoxFuture<'static, Q::PushResult>>,

    /// Cache for BLS public keys to avoid repetitive uncompressing.
    bls_cache: Arc<Mutex<BlsCache>>,

    /// Channel used to communicate additional blocks to the queue.
    /// We use this to wake up the queue and pass in new, unknown blocks
    /// received in the consensus as part of the head requests.
    block_tx: mpsc::Sender<BlockAndSource<N>>,
}

impl<N: Network, Q: LiveSyncQueue<N>> LiveSyncer<N, Q> {
    // Sets up an internal channel to receive blocks and connects it to the queue
    pub fn with_queue(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        mut queue: Q,
        bls_cache: Arc<Mutex<BlsCache>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(MAX_BLOCK_STREAM_BUFFER);
        queue.add_block_stream(ReceiverStream::new(rx));
        Self {
            blockchain,
            network,
            queue,
            pending: Default::default(),
            bls_cache,
            block_tx: tx,
        }
    }

    pub fn queue(&self) -> &Q {
        &self.queue
    }
}

impl<N: Network, Q: LiveSyncQueue<N>> LiveSync<N> for LiveSyncer<N, Q> {
    fn push_block(&mut self, block: Block, block_source: BlockSource<N>) {
        if let Err(e) = self.block_tx.try_send((block, block_source)) {
            error!("Queue not ready to receive data: {}", e);
        }
    }

    fn add_peer(&mut self, peer_id: N::PeerId) {
        self.queue.add_peer(peer_id);
    }

    fn num_peers(&self) -> usize {
        self.queue.num_peers()
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.queue.peers()
    }

    fn state_complete(&self) -> bool {
        self.queue.state_complete()
    }

    fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        self.queue.resolve_block(request)
    }

    fn acceptance_window_size(&self) -> u32 {
        self.queue.acceptance_window_size()
    }
}

impl<N: Network, Q: LiveSyncQueue<N>> Stream for LiveSyncer<N, Q> {
    type Item = LiveSyncEvent<N::PeerId>;

    // Stream that polls for completed block push operations and new sync items from the queue.
    // Completed operations produce events. New items are turned into push tasks.
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        Poll::Ready(loop {
            if let Some(p) = self.pending.front_mut() {
                // We have an item in progress, poll that until it's done
                let item = ready!(p.as_mut().poll(cx));
                self.pending.pop_front();
                if let Some(event) = self.queue.process_push_result(item) {
                    break Some(event);
                }
            } else if let Some(item) = ready!(self.queue.poll_next_unpin(cx)) {
                // No item in progress, but the stream is still going
                self.pending = Q::push_queue_result(
                    Arc::clone(&self.network),
                    self.blockchain.clone(),
                    Arc::clone(&self.bls_cache),
                    item,
                );
            } else {
                // The stream is done
                break None;
            }
        })
    }
}
