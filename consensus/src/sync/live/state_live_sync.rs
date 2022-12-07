use crate::sync::live::block_queue::{BlockTopic, QueuedBlock};
use crate::sync::live::block_request_component::RequestComponent;
use crate::sync::syncer::{LiveSync, LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent};
use futures::future::BoxFuture;
use futures::{FutureExt, Stream, StreamExt};
use nimiq_account::AccountError;
use nimiq_block::Block;
use nimiq_blockchain::{Blockchain, PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::{MsgAcceptance, Network};
use parking_lot::Mutex;
use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};
use tokio::sync::mpsc::{channel as mpsc, Sender as MpscSender};
use tokio::task::spawn_blocking;
use tokio_stream::wrappers::ReceiverStream;

use super::state_queue::{ChunkAndId, QueuedStateChunks, StateQueue};

enum PushOpResult {
    Head(
        Result<PushResult, PushError>,
        Blake2bHash,
        Option<AccountError>,
    ),
    HeadChunk(Blake2bHash, Option<AccountError>),
    Buffered(
        Result<PushResult, PushError>,
        Blake2bHash,
        Option<AccountError>,
    ),
    Missing(
        Result<PushResult, PushError>,
        Vec<Blake2bHash>,
        HashSet<Blake2bHash>,
        Option<AccountError>,
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
    block: Option<Block>,
    chunks: Vec<ChunkAndId<N>>,
    pubsub_id: Option<N::PubsubId>,
) -> (
    Option<Result<PushResult, PushError>>,
    Option<AccountError>,
    Blake2bHash,
) {
    let (push_result, push_chunk_error, block_hash) = spawn_blocking(move || {
        // Update validator keys from BLS public key cache.
        if let Some(ref block) = block {
            block.update_validator_keys(&mut bls_cache.lock());
        }
        match blockchain {
            BlockchainProxy::Full(blockchain) => {
                let (state_hash, block_push_result, block_hash) = if let Some(block) = block {
                    let block_hash = block.hash();
                    let state_hash = block.state_root().clone();
                    let block_push_result = Blockchain::push(blockchain.upgradable_read(), block);
                    if block_push_result.is_err() {
                        return (Some(block_push_result), None, block_hash);
                    }
                    (state_hash, Some(block_push_result), block_hash)
                } else {
                    let block_hash = blockchain.read().state.head_hash.clone();
                    (
                        blockchain.read().state.main_chain.head.state_root().clone(),
                        None,
                        block_hash,
                    )
                };

                for chunk_data in chunks {
                    let blockchain = blockchain.read();
                    let mut txn = blockchain.write_transaction();

                    if let Err(e) = blockchain.state.accounts.commit_chunk(
                        &mut txn,
                        chunk_data.chunk,
                        state_hash.clone(),
                        chunk_data.start_key,
                    ) {
                        txn.abort();
                        return (block_push_result, Some((e, chunk_data.peer_id)), block_hash);
                    }
                    txn.commit();
                }

                (block_push_result, None, block_hash)
            }
            BlockchainProxy::Light(_) => todo!(),
        }
    })
    .await
    .expect("blockchain.push() should not panic");

    if let Some(id) = pubsub_id {
        if let Some(ref push_result) = push_result {
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
            network.validate_message::<BlockTopic>(id, acceptance);
        }
    }

    if let Some((e, _peer_id)) = push_chunk_error {
        log::warn!("Commit chunk for block {} failed: {}", block_hash, e);
        // TODO Ban peer
        return (push_result, Some(e), block_hash);
    }

    (push_result, None, block_hash)
}

pub struct StateLiveSync<N: Network, TReq: RequestComponent<N>> {
    blockchain: BlockchainProxy,

    network: Arc<N>,

    pub state_queue: StateQueue<N, TReq>,

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

impl<N: Network, TReq: RequestComponent<N>> LiveSync<N> for StateLiveSync<N, TReq> {
    fn push_block(&mut self, block: Block, peer_id: N::PeerId, pubsub_id: Option<N::PubsubId>) {
        if let Err(e) = self.block_queue_tx.try_send((block, peer_id, pubsub_id)) {
            error!("Block queue not ready to receive data: {}", e);
        }
    }

    fn add_peer(&mut self, peer_id: N::PeerId) {
        self.state_queue.add_peer(peer_id);
    }

    fn num_peers(&self) -> usize {
        self.state_queue.num_peers()
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.state_queue.peers()
    }
}

impl<N: Network, TReq: RequestComponent<N>> StateLiveSync<N, TReq> {
    pub fn with_block_queue(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        mut state_queue: StateQueue<N, TReq>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
    ) -> StateLiveSync<N, TReq> {
        let (tx, rx) = mpsc(MAX_BLOCK_STREAM_BUFFER);
        state_queue.add_block_stream(ReceiverStream::new(rx));
        StateLiveSync {
            blockchain,
            network,
            state_queue,
            accepted_announcements: 0,
            push_ops: Default::default(),
            bls_cache,
            block_queue_tx: tx,
        }
    }

    pub fn accepted_block_announcements(&self) -> usize {
        self.accepted_announcements
    }

    fn process_queued_block(
        &mut self,
        queued_block: QueuedBlock<N>,
        chunks: Vec<ChunkAndId<N>>,
    ) -> Option<LiveSyncPeerEvent<N::PeerId>> {
        match queued_block {
            QueuedBlock::Head((block, pubsub_id)) => {
                let future = push_single_block(
                    self.blockchain.clone(),
                    Arc::clone(&self.bls_cache),
                    Arc::clone(&self.network),
                    Some(block),
                    chunks,
                    pubsub_id,
                )
                .map(|(push_result, push_chunk_error, block_hash)| {
                    PushOpResult::Head(push_result.unwrap(), block_hash, push_chunk_error)
                });
                self.push_ops.push_back(future.boxed());
            }
            QueuedBlock::Buffered(mut buffered_blocks) => {
                assert_eq!(
                    buffered_blocks.len(),
                    1,
                    "State queue should only return single blocks"
                );
                let (block, pubsub_id) = buffered_blocks.pop().unwrap(); // PITODO unwraps need cleanup
                let future = push_single_block(
                    self.blockchain.clone(),
                    Arc::clone(&self.bls_cache),
                    Arc::clone(&self.network),
                    Some(block),
                    chunks,
                    pubsub_id,
                )
                .map(|(push_result, push_chunk_error, block_hash)| {
                    PushOpResult::Buffered(push_result.unwrap(), block_hash, push_chunk_error)
                });

                self.push_ops.push_back(future.boxed());
            }
            QueuedBlock::Missing(mut blocks) => {
                assert_eq!(
                    blocks.len(),
                    1,
                    "State queue should only return single blocks"
                );
                let block = blocks.pop().unwrap();

                let future = push_single_block(
                    self.blockchain.clone(),
                    Arc::clone(&self.bls_cache),
                    Arc::clone(&self.network),
                    Some(block),
                    chunks,
                    None,
                )
                .map(|(push_result, push_chunk_error, block_hash)| {
                    let mut hash_set = HashSet::new();
                    if push_result.as_ref().unwrap().is_err() {
                        hash_set.insert(block_hash);
                        PushOpResult::Missing(
                            push_result.unwrap(),
                            vec![],
                            hash_set,
                            push_chunk_error,
                        )
                    } else {
                        PushOpResult::Missing(
                            push_result.unwrap(),
                            vec![block_hash],
                            hash_set,
                            push_chunk_error,
                        )
                    }
                });

                self.push_ops.push_back(future.boxed());
            }
            QueuedBlock::TooFarFuture(_, peer_id) => {
                return Some(LiveSyncPeerEvent::AdvancedPeer(peer_id));
            }
            QueuedBlock::TooDistantPast(_, peer_id) => {
                return Some(LiveSyncPeerEvent::OutdatedPeer(peer_id));
            }
        };
        None
    }

    fn poll_block_queue(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<LiveSyncPeerEvent<N::PeerId>>> {
        while let Poll::Ready(Some(queued_chunk)) = self.state_queue.poll_next_unpin(cx) {
            match queued_chunk {
                QueuedStateChunks::StateChunk(block, chunks) => {
                    if let Some(event) = self.process_queued_block(block, chunks) {
                        return Poll::Ready(Some(event));
                    };
                }
                QueuedStateChunks::HeadStateChunk(chunks) => {
                    let future = push_single_block(
                        self.blockchain.clone(),
                        Arc::clone(&self.bls_cache),
                        Arc::clone(&self.network),
                        None,
                        chunks,
                        None,
                    )
                    .map(|(push_result, push_chunk_error, block_hash)| {
                        assert!(
                            push_result.is_none(),
                            "Head state chunk should not have a block being pushed."
                        );
                        PushOpResult::HeadChunk(block_hash, push_chunk_error)
                    });
                    self.push_ops.push_back(future.boxed());
                }
                QueuedStateChunks::TooFarFuture(chunk_data) => {
                    return Poll::Ready(Some(LiveSyncPeerEvent::AdvancedPeer(chunk_data.peer_id)));
                }
                QueuedStateChunks::TooDistantPast(chunk_data) => {
                    return Poll::Ready(Some(LiveSyncPeerEvent::OutdatedPeer(chunk_data.peer_id)));
                }
            };
        }
        Poll::Pending
    }

    fn process_push_results(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<LiveSyncPushEvent>> {
        while let Some(op) = self.push_ops.front_mut() {
            let result = ready!(op.poll_unpin(cx));
            self.push_ops.pop_front().expect("PushOp should be present");

            match result {
                PushOpResult::Head(Ok(result), hash, push_chunk_error) => {
                    self.state_queue.on_block_processed(&hash);
                    if push_chunk_error.is_some() {
                        self.state_queue.reset_chunk_request_chain();
                    }
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        self.accepted_announcements = self.accepted_announcements.saturating_add(1);
                        return Poll::Ready(Some(LiveSyncPushEvent::AcceptedAnnouncedBlock(hash)));
                    }
                }
                PushOpResult::HeadChunk(_hash, push_chunk_error) => {
                    if push_chunk_error.is_some() {
                        self.state_queue.reset_chunk_request_chain();
                    }
                }
                PushOpResult::Buffered(Ok(result), hash, push_chunk_error) => {
                    self.state_queue.on_block_processed(&hash);
                    if push_chunk_error.is_some() {
                        self.state_queue.reset_chunk_request_chain();
                    }
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        return Poll::Ready(Some(LiveSyncPushEvent::AcceptedBufferedBlock(
                            hash,
                            self.state_queue.buffered_blocks_len(),
                        )));
                    }
                }
                PushOpResult::Missing(
                    result,
                    mut adopted_blocks,
                    mut invalid_blocks,
                    push_chunk_error,
                ) => {
                    for hash in &adopted_blocks {
                        self.state_queue.on_block_processed(hash);
                    }
                    for hash in &invalid_blocks {
                        self.state_queue.on_block_processed(hash);
                    }

                    self.state_queue.remove_invalid_blocks(&mut invalid_blocks);
                    if push_chunk_error.is_some() {
                        self.state_queue.reset_chunk_request_chain();
                    }

                    if result.is_ok() && !adopted_blocks.is_empty() {
                        let hash = adopted_blocks.pop().expect("adopted_blocks not empty");
                        return Poll::Ready(Some(LiveSyncPushEvent::ReceivedMissingBlocks(
                            hash,
                            adopted_blocks.len() + 1,
                        )));
                    }
                }
                PushOpResult::Head(Err(result), hash, push_chunk_error)
                | PushOpResult::Buffered(Err(result), hash, push_chunk_error) => {
                    // If there was a blockchain push error, we remove the block from the pending blocks
                    log::trace!("Head push operation failed because of {}", result);
                    self.state_queue.on_block_processed(&hash);
                    if push_chunk_error.is_some() {
                        self.state_queue.reset_chunk_request_chain();
                    }
                    return Poll::Ready(Some(LiveSyncPushEvent::RejectedBlock(hash)));
                }
            }
        }

        Poll::Pending
    }
}

impl<N: Network, TReq: RequestComponent<N>> Stream for StateLiveSync<N, TReq> {
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
