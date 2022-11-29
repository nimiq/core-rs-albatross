use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use futures::{future::BoxFuture, FutureExt, Stream, StreamExt};
use parking_lot::Mutex;
use pin_project::pin_project;
use tokio::{
    sync::mpsc::{channel as mpsc, Sender as MpscSender},
    task::spawn_blocking,
};
use tokio_stream::wrappers::ReceiverStream;

use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::{MsgAcceptance, Network};

use crate::sync::{
    live::{
        block_queue::{BlockHeaderTopic, BlockQueue, BlockTopic, QueuedBlock},
        block_request_component::RequestComponent,
    },
    syncer::{LiveSync, LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent},
};

enum PushOpResult {
    Head(Result<PushResult, PushError>, Blake2bHash),
    Buffered(Result<PushResult, PushError>, Blake2bHash),
    Missing(
        Result<PushResult, PushError>,
        Vec<Blake2bHash>,
        HashSet<Blake2bHash>,
    ),
}

/// The maximum capacity of the external block stream passed into the block queue.
const MAX_BLOCK_STREAM_BUFFER: usize = 256;

/// Pushes a single block into the blockchain.
/// TODO We should limit the number of push operations we queue here.
async fn push_single_block<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    network: Arc<N>,
    block: Block,
    pubsub_id: Option<N::PubsubId>,
    is_head: bool,
    include_body: bool,
) -> PushOpResult {
    let block_hash = block.hash();
    let push_result = spawn_blocking(move || {
        // Update validator keys from BLS public key cache.
        block.update_validator_keys(&mut bls_cache.lock());
        match blockchain {
            BlockchainProxy::Full(blockchain) => {
                Blockchain::push(blockchain.upgradable_read(), block)
            }
            BlockchainProxy::Light(_) => todo!(),
        }
    })
    .await
    .expect("blockchain.push() should not panic");

    let acceptance = match &push_result {
        Ok(result) => match result {
            PushResult::Known | PushResult::Extended | PushResult::Rebranched => {
                MsgAcceptance::Accept
            }
            PushResult::Forked | PushResult::Ignored => MsgAcceptance::Ignore,
        },
        Err(_) => {
            // TODO Ban peer
            MsgAcceptance::Reject
        }
    };

    if let Some(id) = pubsub_id {
        if include_body {
            network.validate_message::<BlockTopic>(id, acceptance);
        } else {
            network.validate_message::<BlockHeaderTopic>(id, acceptance);
        }
    }

    if is_head {
        PushOpResult::Head(push_result, block_hash)
    } else {
        PushOpResult::Buffered(push_result, block_hash)
    }
}

/// Pushes a sequence of blocks to the blockchain.
/// This case is different from pushing single blocks in a for loop,
/// because an invalid block automatically invalidates the remainder of the sequence.
///
/// TODO We should limit the number of push operations we queue here.
async fn push_missing_blocks(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    blocks: Vec<Block>,
) -> PushOpResult {
    let mut block_iter = blocks.into_iter();

    // Hashes of adopted blocks
    let mut adopted_blocks = Vec::new();

    // Hashes of invalid blocks
    let mut invalid_blocks = HashSet::new();

    // Initialize push_result to some random value.
    // It is always overwritten in the first loop iteration.
    let mut push_result = Err(PushError::Orphan);

    // Try to push blocks, until we encounter an invalid block.
    #[allow(clippy::while_let_on_iterator)]
    while let Some(block) = block_iter.next() {
        let block_hash = block.hash();

        log::debug!("Pushing block {} from missing blocks response", block);
        let blockchain2 = blockchain.clone();
        let bls_cache2 = Arc::clone(&bls_cache);
        push_result = spawn_blocking(move || {
            // Update validator keys from BLS public key cache.
            block.update_validator_keys(&mut bls_cache2.lock());
            match blockchain2 {
                BlockchainProxy::Full(blockchain) => {
                    Blockchain::push(blockchain.upgradable_read(), block)
                }
                BlockchainProxy::Light(_) => todo!(),
            }
        })
        .await
        .expect("blockchain.push() should not panic");
        match &push_result {
            Err(e) => {
                log::warn!("Failed to push missing block {}: {}", block_hash, e);
                invalid_blocks.insert(block_hash);
                break;
            }
            Ok(_) => {
                adopted_blocks.push(block_hash);
            }
        }
    }

    // If there are remaining blocks in the iterator, those are invalid.
    for block in block_iter {
        invalid_blocks.insert(block.hash());
    }

    PushOpResult::Missing(push_result, adopted_blocks, invalid_blocks)
}

#[pin_project]
pub struct BlockLiveSync<N: Network, TReq: RequestComponent<N>> {
    blockchain: BlockchainProxy,

    network: Arc<N>,

    #[pin]
    pub block_queue: BlockQueue<N, TReq>,

    /// The number of extended blocks through announcements.
    accepted_announcements: usize,

    /// Vector of pending `blockchain.push()` operations.
    push_ops: VecDeque<BoxFuture<'static, PushOpResult>>,

    /// Cache for BLS public keys to avoid repetitive uncompressing.
    bls_cache: Arc<Mutex<PublicKeyCache>>,

    /// Channel used to communicate additional blocks to the block queue.
    /// We use this to wake up the block queue and pass in new, unknown blocks
    /// received in the consensus as part of the head requests.
    block_queue_tx: MpscSender<(Block, N::PeerId, Option<N::PubsubId>)>,
}

impl<N: Network, TReq: RequestComponent<N>> LiveSync<N> for BlockLiveSync<N, TReq> {
    fn push_block(&mut self, block: Block, peer_id: N::PeerId, pubsub_id: Option<N::PubsubId>) {
        if let Err(e) = self.block_queue_tx.try_send((block, peer_id, pubsub_id)) {
            error!("Block queue not ready to receive data: {}", e);
        }
    }

    fn add_peer(&mut self, peer_id: N::PeerId) {
        self.request_component_mut().add_peer(peer_id);
    }

    fn num_peers(&self) -> usize {
        self.block_queue.request_component().num_peers()
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.block_queue.request_component().peers()
    }
}

impl<N: Network, TReq: RequestComponent<N>> BlockLiveSync<N, TReq> {
    pub fn with_block_queue(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        mut block_queue: BlockQueue<N, TReq>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
    ) -> BlockLiveSync<N, TReq> {
        let (tx, rx) = mpsc(MAX_BLOCK_STREAM_BUFFER);
        block_queue.add_block_stream(ReceiverStream::new(rx));
        BlockLiveSync {
            blockchain,
            network,
            block_queue,
            accepted_announcements: 0,
            push_ops: Default::default(),
            bls_cache,
            block_queue_tx: tx,
        }
    }

    pub fn num_peers(&self) -> usize {
        self.block_queue.request_component().num_peers()
    }

    pub fn accepted_block_announcements(&self) -> usize {
        self.accepted_announcements
    }

    pub fn request_component(&self) -> &TReq {
        self.block_queue.request_component()
    }

    pub fn request_component_mut(&mut self) -> &mut TReq {
        self.block_queue.request_component_mut()
    }

    fn poll_block_queue(
        self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<LiveSyncPeerEvent<N::PeerId>>> {
        let mut this = self.project();

        while let Poll::Ready(Some(queued_block)) = this.block_queue.poll_next_unpin(cx) {
            let include_body = this.block_queue.include_micro_bodies();
            match queued_block {
                QueuedBlock::Head((block, pubsub_id)) => {
                    let future = push_single_block(
                        this.blockchain.clone(),
                        Arc::clone(this.bls_cache),
                        Arc::clone(this.network),
                        block,
                        pubsub_id,
                        true,
                        include_body,
                    );
                    this.push_ops.push_back(future.boxed());
                }
                QueuedBlock::Buffered(buffered_blocks) => {
                    for (block, pubsub_id) in buffered_blocks {
                        let future = push_single_block(
                            this.blockchain.clone(),
                            Arc::clone(this.bls_cache),
                            Arc::clone(this.network),
                            block,
                            pubsub_id,
                            true,
                            include_body,
                        );
                        this.push_ops.push_back(future.boxed());
                    }
                }
                QueuedBlock::Missing(blocks) => {
                    let future = push_missing_blocks(
                        this.blockchain.clone(),
                        Arc::clone(this.bls_cache),
                        blocks,
                    );
                    this.push_ops.push_back(future.boxed());
                }
                QueuedBlock::TooFarFuture(_, peer_id) => {
                    return Poll::Ready(Some(LiveSyncPeerEvent::AdvancedPeer(peer_id)));
                }
                QueuedBlock::TooDistantPast(_, peer_id) => {
                    return Poll::Ready(Some(LiveSyncPeerEvent::OutdatedPeer(peer_id)));
                }
            };
        }
        Poll::Pending
    }

    fn process_push_results(
        self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<LiveSyncPushEvent>> {
        let mut this = self.project();

        while let Some(op) = this.push_ops.front_mut() {
            let result = ready!(op.poll_unpin(cx));
            this.push_ops.pop_front().expect("PushOp should be present");

            match result {
                PushOpResult::Head(Ok(result), hash) => {
                    this.block_queue.on_block_processed(&hash);
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        *this.accepted_announcements =
                            this.accepted_announcements.saturating_add(1);
                        return Poll::Ready(Some(LiveSyncPushEvent::AcceptedAnnouncedBlock(hash)));
                    }
                }
                PushOpResult::Buffered(Ok(result), hash) => {
                    this.block_queue.on_block_processed(&hash);
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        return Poll::Ready(Some(LiveSyncPushEvent::AcceptedBufferedBlock(
                            hash,
                            this.block_queue.buffered_blocks_len(),
                        )));
                    }
                }
                PushOpResult::Missing(result, mut adopted_blocks, mut invalid_blocks) => {
                    for hash in &adopted_blocks {
                        this.block_queue.on_block_processed(hash);
                    }
                    for hash in &invalid_blocks {
                        this.block_queue.on_block_processed(hash);
                    }

                    this.block_queue.remove_invalid_blocks(&mut invalid_blocks);

                    if result.is_ok() && !adopted_blocks.is_empty() {
                        let hash = adopted_blocks.pop().expect("adopted_blocks not empty");
                        return Poll::Ready(Some(LiveSyncPushEvent::ReceivedMissingBlocks(
                            hash,
                            adopted_blocks.len() + 1,
                        )));
                    }
                }
                PushOpResult::Head(Err(result), hash)
                | PushOpResult::Buffered(Err(result), hash) => {
                    // If there was a blockchain push error, we remove the block from the pending blocks
                    log::trace!("Head push operation failed because of {}", result);
                    this.block_queue.on_block_processed(&hash);
                    return Poll::Ready(Some(LiveSyncPushEvent::RejectedBlock(hash)));
                }
            }
        }

        Poll::Pending
    }
}

impl<N: Network, TReq: RequestComponent<N>> Stream for BlockLiveSync<N, TReq> {
    type Item = LiveSyncEvent<N::PeerId>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(result)) = self.as_mut().poll_block_queue(cx) {
            return Poll::Ready(Some(LiveSyncEvent::PeerEvent(result)));
        }

        if let Poll::Ready(Some(result)) = self.process_push_results(cx) {
            return Poll::Ready(Some(LiveSyncEvent::PushEvent(result)));
        }
        Poll::Pending
    }
}
