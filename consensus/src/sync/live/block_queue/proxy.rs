use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context, Poll},
};

use futures::{future::BoxFuture, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use nimiq_utils::spawn;
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::{
    consensus::ResolveBlockRequest,
    sync::{
        live::{
            block_queue::{
                live_sync::PushOpResult, queue::BlockQueue, BlockAndSource, BlockStream,
                GossipSubBlockStream, QueuedBlock,
            },
            queue::{LiveSyncQueue, QueueConfig},
        },
        peer_list::PeerList,
        syncer::LiveSyncEvent,
    },
};

pub struct BlockQueueProxy<N: Network> {
    queue: Arc<Mutex<BlockQueue<N>>>,
    receiver: UnboundedReceiver<QueuedBlock<N>>,
}

impl<N: Network> BlockQueueProxy<N> {
    pub async fn new(network: Arc<N>, blockchain: BlockchainProxy, config: QueueConfig) -> Self {
        let queue = BlockQueue::new(network, blockchain, config).await;
        Self::with_queue(queue)
    }

    pub fn with_gossipsub_block_stream(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        block_stream: GossipSubBlockStream<N>,
        config: QueueConfig,
    ) -> Self {
        let queue =
            BlockQueue::with_gossipsub_block_stream(blockchain, network, block_stream, config);
        Self::with_queue(queue)
    }

    pub fn with_block_stream(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        block_stream: BlockStream<N>,
        config: QueueConfig,
    ) -> Self {
        let queue = BlockQueue::with_block_stream(blockchain, network, block_stream, config);
        Self::with_queue(queue)
    }

    pub fn with_queue(queue: BlockQueue<N>) -> Self {
        let queue = Arc::new(Mutex::new(queue));

        let (sender, receiver) = unbounded_channel();

        let future = BlockQueueFuture {
            queue: Arc::downgrade(&queue),
            sender,
        };
        spawn(future);

        Self { queue, receiver }
    }

    pub fn on_block_processed(&mut self, block_hash: &Blake2bHash) {
        self.queue.lock().on_block_processed(block_hash);
    }

    pub fn remove_invalid_blocks(&mut self, invalid_blocks: &mut HashSet<Blake2bHash>) {
        self.queue.lock().remove_invalid_blocks(invalid_blocks);
    }

    pub fn buffered_blocks(&self) -> Vec<(u32, Vec<Block>)> {
        self.queue.lock().buffered_blocks()
    }

    pub fn num_buffered_blocks(&self) -> usize {
        self.queue.lock().num_buffered_blocks()
    }

    pub(crate) fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        self.queue.lock().peer_list()
    }
}

impl<N: Network> LiveSyncQueue<N> for BlockQueueProxy<N> {
    type QueueResult = QueuedBlock<N>;
    type PushResult = PushOpResult<N>;

    fn push_queue_result(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        result: Self::QueueResult,
    ) -> VecDeque<BoxFuture<'static, Self::PushResult>> {
        BlockQueue::push_queue_result(network, blockchain, bls_cache, result)
    }

    fn process_push_result(&mut self, item: Self::PushResult) -> Option<LiveSyncEvent<N::PeerId>> {
        self.queue.lock().process_push_result(item)
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.queue.lock().peers()
    }

    fn num_peers(&self) -> usize {
        self.queue.lock().num_peers()
    }

    fn add_peer(&self, peer_id: N::PeerId) {
        self.queue.lock().add_peer(peer_id)
    }

    fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = BlockAndSource<N>> + Send + 'static,
    {
        self.queue.lock().add_block_stream(block_stream)
    }

    fn include_body(&self) -> bool {
        self.queue.lock().include_body()
    }

    fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        self.queue.lock().resolve_block(request)
    }
}

impl<N: Network> Stream for BlockQueueProxy<N> {
    type Item = QueuedBlock<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

struct BlockQueueFuture<N: Network> {
    queue: Weak<Mutex<BlockQueue<N>>>,
    sender: UnboundedSender<QueuedBlock<N>>,
}

impl<N: Network> Future for BlockQueueFuture<N> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let queue = match self.queue.upgrade() {
            Some(queue) => queue,
            None => return Poll::Ready(()),
        };

        while let Poll::Ready(event) = queue.lock().poll_next_unpin(cx) {
            match event {
                Some(queued_block) => {
                    if let Err(error) = self.sender.send(queued_block) {
                        error!(%error, "Failed to dispatch queued block");
                    }
                }
                None => return Poll::Ready(()),
            }
        }
        Poll::Pending
    }
}
