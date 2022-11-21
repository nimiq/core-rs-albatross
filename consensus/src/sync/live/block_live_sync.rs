use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use futures::{future::BoxFuture, FutureExt, Stream, StreamExt};
use parking_lot::Mutex;
use pin_project::pin_project;
use tokio::task::spawn_blocking;

use nimiq_block::{Block, BlockHeaderTopic, BlockTopic};
#[cfg(feature = "full")]
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::{MsgAcceptance, Network};

use crate::sync::{
    live::{
        block_queue::{BlockQueue, QueuedBlock},
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
}

impl<N: Network, TReq: RequestComponent<N>> LiveSync<N> for BlockLiveSync<N, TReq> {
    fn on_block_announced(
        &mut self,
        block: Block,
        peer_id: N::PeerId,
        pubsub_id: Option<N::PubsubId>,
    ) {
        self.block_queue
            .on_block_announced(block, peer_id, pubsub_id);
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
    pub fn new(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        block_queue: BlockQueue<N, TReq>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
    ) -> BlockLiveSync<N, TReq> {
        BlockLiveSync {
            blockchain,
            network,
            block_queue,
            accepted_announcements: 0,
            push_ops: Default::default(),
            bls_cache,
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

impl<N: Network, TReq: RequestComponent<N>> BlockLiveSync<N, TReq> {
    fn poll_block_queue(
        self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<LiveSyncPeerEvent<N::PeerId>>> {
        let mut this = self.project();

        while let Poll::Ready(Some(queued_block)) = this.block_queue.poll_next_unpin(cx) {
            let blockchain1 = this.blockchain.clone();
            let bls_cache1 = Arc::clone(this.bls_cache);
            let network1 = Arc::clone(this.network);
            let include_micro_bodies = this.block_queue.include_micro_bodies();

            let is_head = matches!(queued_block, QueuedBlock::Head(..));
            match queued_block {
                QueuedBlock::Head((block, pubsub_id))
                | QueuedBlock::Buffered((block, pubsub_id)) => {
                    let future = async move {
                        let block_hash = block.hash();
                        let push_result = spawn_blocking(move || {
                            // Update validator keys from BLS public key cache.
                            block.update_validator_keys(&mut bls_cache1.lock());
                            match blockchain1 {
                                #[cfg(feature = "full")]
                                BlockchainProxy::Full(blockchain) => {
                                    Blockchain::push(blockchain.upgradable_read(), block)
                                }
                                BlockchainProxy::Light(blockchain) => {
                                    LightBlockchain::push(blockchain.upgradable_read(), block)
                                }
                            }
                        })
                        .await
                        .expect("blockchain.push() should not panic");

                        let acceptance = match &push_result {
                            Ok(result) => match result {
                                PushResult::Known
                                | PushResult::Extended
                                | PushResult::Rebranched => MsgAcceptance::Accept,
                                PushResult::Forked | PushResult::Ignored => MsgAcceptance::Ignore,
                            },
                            Err(_) => {
                                // TODO Ban peer
                                MsgAcceptance::Reject
                            }
                        };

                        if let Some(id) = pubsub_id {
                            if include_micro_bodies {
                                network1.validate_message::<BlockTopic>(id, acceptance);
                            } else {
                                network1.validate_message::<BlockHeaderTopic>(id, acceptance);
                            }
                        }

                        if is_head {
                            PushOpResult::Head(push_result, block_hash)
                        } else {
                            PushOpResult::Buffered(push_result, block_hash)
                        }
                    };
                    this.push_ops.push_back(future.boxed());
                }
                QueuedBlock::Missing(blocks) => {
                    let future = async move {
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
                            let blockchain2 = blockchain1.clone();
                            let bls_cache2 = Arc::clone(&bls_cache1);
                            push_result = spawn_blocking(move || {
                                // Update validator keys from BLS public key cache.
                                block.update_validator_keys(&mut bls_cache2.lock());
                                match blockchain2 {
                                    #[cfg(feature = "full")]
                                    BlockchainProxy::Full(blockchain) => {
                                        Blockchain::push(blockchain.upgradable_read(), block)
                                    }
                                    BlockchainProxy::Light(blockchain) => {
                                        LightBlockchain::push(blockchain.upgradable_read(), block)
                                    }
                                }
                            })
                            .await
                            .expect("blockchain.push() should not panic");
                            match &push_result {
                                Err(e) => {
                                    log::warn!(
                                        "Failed to push missing block {}: {}",
                                        block_hash,
                                        e
                                    );
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
                    };
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
                    this.block_queue.on_block_accepted();
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        *this.accepted_announcements =
                            this.accepted_announcements.saturating_add(1);
                        return Poll::Ready(Some(LiveSyncPushEvent::AcceptedAnnouncedBlock(hash)));
                    }
                }
                PushOpResult::Buffered(Ok(result), hash) => {
                    this.block_queue.on_block_processed(&hash);
                    this.block_queue.on_block_accepted();
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
                    this.block_queue.on_block_accepted();

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
