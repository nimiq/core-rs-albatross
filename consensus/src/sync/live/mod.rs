use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use futures::{future::BoxFuture, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_network_interface::network::Network;
use parking_lot::Mutex;
use tokio::sync::mpsc::{channel as mpsc, Sender as MpscSender};
use tokio_stream::wrappers::ReceiverStream;

#[cfg(feature = "full")]
use self::state_queue::StateQueue;
use self::{block_queue::BlockQueue, queue::LiveSyncQueue};
use super::syncer::{LiveSync, LiveSyncEvent};
use crate::consensus::ResolveBlockRequest;

pub mod block_queue;
#[cfg(feature = "full")]
pub mod diff_queue;
pub mod queue;
#[cfg(feature = "full")]
pub mod state_queue;

pub type BlockLiveSync<N> = LiveSyncer<N, BlockQueue<N>>;
#[cfg(feature = "full")]
pub type StateLiveSync<N> = LiveSyncer<N, StateQueue<N>>;

/// The maximum capacity of the external block stream passed into the block queue.
const MAX_BLOCK_STREAM_BUFFER: usize = 256;

pub struct LiveSyncer<N: Network, Q: LiveSyncQueue<N>> {
    blockchain: BlockchainProxy,

    network: Arc<N>,

    queue: Q,

    /// Vector of pending push operations.
    pending: VecDeque<BoxFuture<'static, Q::PushResult>>,

    /// Cache for BLS public keys to avoid repetitive uncompressing.
    bls_cache: Arc<Mutex<PublicKeyCache>>,

    /// Channel used to communicate additional blocks to the queue.
    /// We use this to wake up the queue and pass in new, unknown blocks
    /// received in the consensus as part of the head requests.
    block_tx: MpscSender<(Block, N::PeerId, Option<N::PubsubId>)>,
}

impl<N: Network, Q: LiveSyncQueue<N>> LiveSyncer<N, Q> {
    pub fn with_queue(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        mut queue: Q,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
    ) -> Self {
        let (tx, rx) = mpsc(MAX_BLOCK_STREAM_BUFFER);
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
    fn push_block(&mut self, block: Block, peer_id: N::PeerId, pubsub_id: Option<N::PubsubId>) {
        if let Err(e) = self.block_tx.try_send((block, peer_id, pubsub_id)) {
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
}

impl<N: Network, Q: LiveSyncQueue<N>> Stream for LiveSyncer<N, Q> {
    type Item = LiveSyncEvent<N::PeerId>;

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
                    self.queue.include_micro_bodies(),
                );
            } else {
                // The stream is done
                break None;
            }
        })
    }
}
