use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{future::BoxFuture, ready, stream::BoxStream, FutureExt, Stream, StreamExt};
use nimiq_bls::cache::PublicKeyCache;
use parking_lot::{Mutex, RwLock};
use pin_project::pin_project;
use tokio::task::spawn_blocking;

use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Direction};
use nimiq_blockchain::{Blockchain, PushError, PushResult};
use nimiq_hash::Blake2bHash;
use nimiq_macros::store_waker;
use nimiq_network_interface::network::{MsgAcceptance, Network, PubsubId, Topic};
use nimiq_primitives::policy;

use crate::sync::request_component::RequestComponentEvent;

use super::request_component::RequestComponent;

#[derive(Clone, Debug, Default)]
pub struct BlockTopic;

impl Topic for BlockTopic {
    type Item = Block;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "blocks";
    const VALIDATE: bool = true;
}

pub type BlockStream<N> = BoxStream<'static, (Block, <N as Network>::PubsubId)>;
type BlockAndId<N> = (Block, Option<<N as Network>::PubsubId>);

#[derive(Clone, Debug)]
pub enum BlockQueueEvent {
    AcceptedAnnouncedBlock(Blake2bHash),
    AcceptedBufferedBlock(Blake2bHash, usize),
    ReceivedMissingBlocks(Blake2bHash, usize),
    RejectedBlock(Blake2bHash),
}

#[derive(Clone, Debug)]
pub struct BlockQueueConfig {
    /// Buffer size limit
    pub buffer_max: usize,

    /// How many blocks ahead we will buffer.
    pub window_max: u32,
}

impl Default for BlockQueueConfig {
    fn default() -> Self {
        Self {
            buffer_max: 4 * policy::BLOCKS_PER_BATCH as usize,
            window_max: 2 * policy::BLOCKS_PER_BATCH,
        }
    }
}

struct Inner<N: Network, TReq: RequestComponent<N>> {
    /// Configuration for the block queue
    config: BlockQueueConfig,

    /// Reference to the block chain
    blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network
    network: Arc<N>,

    /// The Peer Tracking and Request Component.
    pub request_component: TReq,

    /// Buffered blocks - `block_height -> block_hash -> BlockAndId`.
    /// There can be multiple blocks at a height if there are forks.
    buffer: BTreeMap<u32, HashMap<Blake2bHash, BlockAndId<N>>>,

    /// Vector of pending `blockchain.push()` operations.
    push_ops: VecDeque<BoxFuture<'static, PushOpResult>>,

    /// Hashes of blocks that are pending to be pushed to the chain.
    pending_blocks: BTreeSet<Blake2bHash>,

    waker: Option<Waker>,

    /// The block number of the latest macro block. We prune the block buffer when it changes.
    current_macro_height: u32,

    /// Cache for BLS public keys to avoid repetitive uncompressing.
    bls_cache: Arc<Mutex<PublicKeyCache>>,
}

enum PushOpResult {
    Head(Result<PushResult, PushError>, Blake2bHash),
    Buffered(Result<PushResult, PushError>, Blake2bHash),
    Missing(
        Result<PushResult, PushError>,
        Vec<Blake2bHash>,
        HashSet<Blake2bHash>,
    ),
}

impl<N: Network, TReq: RequestComponent<N>> Inner<N, TReq> {
    /// Handles a block announcement.
    fn on_block_announced(
        &mut self,
        block: Block,
        peer_id: N::PeerId,
        pubsub_id: Option<<N as Network>::PubsubId>,
    ) {
        // Ignore blocks that we already know.
        let blockchain = self.blockchain.read();
        if blockchain.contains(&block.hash(), true) {
            return;
        }

        let parent_known = blockchain.contains(block.parent_hash(), true);
        let head_height = blockchain.block_number();
        drop(blockchain);

        // Check if a macro block boundary was passed. If so prune the block buffer.
        let macro_height = policy::last_macro_block(head_height);
        if macro_height > self.current_macro_height {
            self.current_macro_height = macro_height;
            self.prune_buffer();
        }

        let block_number = block.block_number();
        if parent_known {
            // New head or fork block.
            // TODO We should limit the number of push operations we queue here.
            self.push_block(block, pubsub_id, PushOpResult::Head);
        } else if block_number > head_height + self.config.window_max {
            log::warn!(
                "Discarding block {} outside of buffer window (max {})",
                block,
                head_height + self.config.window_max,
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);

            if self.network.has_peer(peer_id) {
                self.request_component.put_peer_into_sync_mode(peer_id);
            }
        } else if self.buffer.len() >= self.config.buffer_max {
            log::warn!(
                "Discarding block {}, buffer full (max {})",
                block,
                self.buffer.len(),
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);
        } else if block_number <= macro_height {
            // Block is from a previous batch/epoch, discard it.
            log::warn!(
                "Discarding block {}, we're already at macro block #{}",
                block,
                macro_height
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);
        } else {
            // Block is inside the buffer window, put it in the buffer.
            self.buffer_and_request_missing_blocks(block, pubsub_id);
        }
    }

    fn buffer_and_request_missing_blocks(&mut self, block: Block, pubsub_id: Option<N::PubsubId>) {
        // Make sure that block_number is positive as we subtract from it later on.
        let block_number = block.block_number();
        if block_number == 0 {
            return;
        }

        let parent_hash = block.parent_hash().clone();

        // Insert block into buffer. If we already know the block, we're done.
        let block_known = self.insert_block_into_buffer(block, pubsub_id);
        log::trace!("Buffering block #{}, known={}", block_number, block_known);
        if block_known {
            return;
        }

        // If the parent of this block is already in the buffer, we're done.
        let parent_buffered = self.is_block_buffered(block_number - 1, &parent_hash);
        log::trace!(
            "Parent of block #{} buffered={}",
            block_number,
            parent_buffered
        );
        if parent_buffered {
            return;
        }

        // If the parent of this block is already being pushed, we're done.
        let parent_pending = self.pending_blocks.contains(&parent_hash);
        log::trace!(
            "Parent of block #{} pending={}",
            block_number,
            parent_pending
        );
        if parent_pending {
            return;
        }

        // We don't know the predecessor of this block, request it.
        self.request_missing_blocks(block_number - 1, parent_hash);
    }

    fn insert_block_into_buffer(&mut self, block: Block, pubsub_id: Option<N::PubsubId>) -> bool {
        self.buffer
            .entry(block.block_number())
            .or_default()
            .insert(block.hash(), (block, pubsub_id))
            .is_some()
    }

    fn is_block_buffered(&self, block_number: u32, hash: &Blake2bHash) -> bool {
        self.buffer
            .get(&block_number)
            .map_or(false, |blocks| blocks.contains_key(hash))
    }

    fn request_missing_blocks(&mut self, block_number: u32, block_hash: Blake2bHash) {
        let blockchain = self.blockchain.read();
        let head_hash = blockchain.head_hash();
        let head_height = blockchain.block_number();
        let macro_height = policy::last_macro_block(head_height);

        log::debug!(
            block_number,
            %block_hash,
            %head_hash,
            macro_height,
            "Requesting missing blocks",
        );

        // Get block locators. The blocks returned by `get_blocks` do *not* include the start block.
        // FIXME We don't want to send the full batch as locators here.
        let block_locators = blockchain
            .chain_store
            .get_blocks(
                &head_hash,
                head_height - macro_height,
                false,
                Direction::Backward,
                None,
            )
            .into_iter()
            .map(|block| block.hash());

        // Prepend our current head hash.
        let block_locators = vec![head_hash].into_iter().chain(block_locators).collect();

        // FIXME Send missing blocks request to the peer that announced the block (first).
        self.request_component
            .request_missing_blocks(block_hash, block_locators);
    }

    fn on_missing_blocks_received(&mut self, blocks: Vec<Block>) {
        if blocks.is_empty() {
            log::debug!("Received empty missing blocks response");
            return;
        }

        // FIXME Sanity-check blocks

        // Check if we can push the missing blocks. This might not be the case if the reference
        // block used in the request is from a batch that we have not adopted yet.
        let parent_known = self
            .blockchain
            .read()
            .contains(blocks[0].parent_hash(), true);
        if !parent_known {
            // We can't push the blocks right away, put them in the buffer.
            // Recursively request missing blocks for the first block we received.
            let mut blocks = blocks.into_iter();
            let first_block = blocks.next().unwrap();
            self.buffer_and_request_missing_blocks(first_block, None);

            // Store the remaining blocks in the buffer.
            for block in blocks {
                self.insert_block_into_buffer(block, None);
            }

            return;
        }

        // Push missing blocks to the chain.
        self.pending_blocks
            .extend(blocks.iter().map(|block| block.hash()));

        let blockchain = Arc::clone(&self.blockchain);
        let cache = Arc::clone(&self.bls_cache);
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

                let blockchain1 = Arc::clone(&blockchain);
                let cache1 = Arc::clone(&cache);
                push_result = spawn_blocking(move || {
                    // Update validator keys from BLS public key cache.
                    block.update_validator_keys(&mut cache1.lock());
                    Blockchain::push(blockchain1.upgradable_read(), block)
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
        };

        self.push_ops.push_back(future.boxed());
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// Pushes a single block to the blockchain.
    fn push_block<F>(&mut self, block: Block, pubsub_id: Option<<N as Network>::PubsubId>, op: F)
    where
        F: Fn(Result<PushResult, PushError>, Blake2bHash) -> PushOpResult + Send + 'static,
    {
        let block_hash = block.hash();
        if !self.pending_blocks.insert(block_hash.clone()) {
            // The block is already pending, so no need to add another future to push it.
            return;
        }

        let blockchain = Arc::clone(&self.blockchain);
        let cache = Arc::clone(&self.bls_cache);
        let network = Arc::clone(&self.network);
        let future = async move {
            let push_result = spawn_blocking(move || {
                // Update validator keys from BLS public key cache.
                block.update_validator_keys(&mut cache.lock());
                Blockchain::push(blockchain.upgradable_read(), block)
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

            // Let the network layer know if it should relay the message this block came from.
            if let Some(id) = pubsub_id {
                network.validate_message::<BlockTopic>(id, acceptance);
            }

            op(push_result, block_hash)
        };

        // TODO We should limit the number of push operations we queue here.
        self.push_ops.push_back(future.boxed());
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn push_buffered(&mut self) {
        let mut blocks_to_push = vec![];
        {
            let blockchain = self.blockchain.read();
            self.buffer.retain(|_, blocks| {
                // Push all blocks with a known parent to the chain.
                blocks.retain(|_, (block, pubsub_id)| {
                    let push = blockchain.contains(block.parent_hash(), true);
                    if push {
                        blocks_to_push.push((block.clone(), pubsub_id.clone()));
                    }
                    !push
                });

                // Remove buffer entry if there are no blocks left.
                !blocks.is_empty()
            });
        }

        for (block, pubsub_id) in blocks_to_push {
            self.push_block(block, pubsub_id, PushOpResult::Buffered);
        }
    }

    fn remove_invalid_blocks(&mut self, mut invalid_blocks: HashSet<Blake2bHash>) {
        if invalid_blocks.is_empty() {
            return;
        }

        // Iterate over block buffer, remove element if no blocks remain at that height.
        self.buffer.retain(|_block_number, blocks| {
            // Iterate over all blocks at the current height, remove block if parent is invalid
            blocks.retain(|hash, (block, pubsub_id)| {
                if invalid_blocks.contains(block.parent_hash()) {
                    log::trace!("Removing block because parent is invalid: {}", hash);
                    invalid_blocks.insert(hash.clone());

                    if let Some(id) = pubsub_id {
                        self.network
                            .validate_message::<BlockTopic>(id.clone(), MsgAcceptance::Reject);
                    }

                    false
                } else {
                    true
                }
            });
            !blocks.is_empty()
        });
    }

    fn prune_buffer(&mut self) {
        self.buffer.retain(|&block_number, blocks| {
            // Remove all entries from the block buffer that precede `current_macro_height`.
            if block_number > self.current_macro_height {
                return true;
            }
            // Tell gossipsub to ignore the removed blocks.
            for (_, pubsub_id) in blocks.values() {
                // Inline `report_validation_result` here, because it solves the borrow issue:
                if let Some(id) = pubsub_id {
                    self.network
                        .validate_message::<BlockTopic>(id.clone(), MsgAcceptance::Ignore);
                }
            }
            false
        });
    }

    #[inline]
    fn report_validation_result(
        &self,
        pubsub_id: Option<<N as Network>::PubsubId>,
        acceptance: MsgAcceptance,
    ) {
        if let Some(id) = pubsub_id {
            self.network.validate_message::<BlockTopic>(id, acceptance);
        }
    }
}

impl<N: Network, TReq: RequestComponent<N>> Stream for Inner<N, TReq> {
    type Item = BlockQueueEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        store_waker!(self, waker, cx);

        // Read all the responses we got for our missing blocks requests.
        loop {
            match self.request_component.poll_next_unpin(cx) {
                Poll::Ready(Some(RequestComponentEvent::ReceivedBlocks(blocks))) => {
                    self.on_missing_blocks_received(blocks);
                }
                Poll::Ready(None) => unreachable!(),
                Poll::Pending => break,
            }
        }

        while let Some(op) = self.push_ops.front_mut() {
            let result = ready!(op.poll_unpin(cx));
            self.push_ops.pop_front().expect("PushOp should be present");

            match result {
                PushOpResult::Head(Ok(result), hash) => {
                    self.pending_blocks.remove(&hash);
                    self.push_buffered();
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        return Poll::Ready(Some(BlockQueueEvent::AcceptedAnnouncedBlock(hash)));
                    }
                }
                PushOpResult::Buffered(Ok(result), hash) => {
                    self.pending_blocks.remove(&hash);
                    self.push_buffered();
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        return Poll::Ready(Some(BlockQueueEvent::AcceptedBufferedBlock(
                            hash,
                            self.buffer.len(),
                        )));
                    }
                }
                PushOpResult::Missing(result, mut adopted_blocks, invalid_blocks) => {
                    for hash in &adopted_blocks {
                        self.pending_blocks.remove(hash);
                    }
                    for hash in &invalid_blocks {
                        self.pending_blocks.remove(hash);
                    }

                    self.remove_invalid_blocks(invalid_blocks);
                    self.push_buffered();

                    if result.is_ok() && !adopted_blocks.is_empty() {
                        let hash = adopted_blocks.pop().expect("adopted_blocks not empty");
                        return Poll::Ready(Some(BlockQueueEvent::ReceivedMissingBlocks(
                            hash,
                            adopted_blocks.len() + 1,
                        )));
                    }
                }

                PushOpResult::Head(Err(result), hash) => {
                    // If there was a blockchain push error, we remove the block from the pending blocks
                    log::trace!("Head push operation failed because of {}", result);
                    self.pending_blocks.remove(&hash);
                    return Poll::Ready(Some(BlockQueueEvent::RejectedBlock(hash)));
                }

                PushOpResult::Buffered(Err(result), hash) => {
                    // If there was a blockchain push error, we remove the block from the pending blocks
                    log::trace!("Buffered push operation failed because of {}", result);
                    self.pending_blocks.remove(&hash);
                    return Poll::Ready(Some(BlockQueueEvent::RejectedBlock(hash)));
                }
            };
        }

        Poll::Pending
    }
}

#[pin_project]
pub struct BlockQueue<N: Network, TReq: RequestComponent<N>> {
    /// The blocks received via gossipsub.
    #[pin]
    block_stream: BlockStream<N>,

    /// The inner state of the block queue.
    #[pin]
    inner: Inner<N, TReq>,

    /// The number of extended blocks through announcements.
    accepted_announcements: usize,
}

impl<N: Network, TReq: RequestComponent<N>> BlockQueue<N, TReq> {
    pub async fn new(
        config: BlockQueueConfig,
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        request_component: TReq,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
    ) -> Self {
        let block_stream = network.subscribe::<BlockTopic>().await.unwrap().boxed();

        Self::with_block_stream(
            config,
            blockchain,
            network,
            request_component,
            block_stream,
            bls_cache,
        )
    }

    pub fn with_block_stream(
        config: BlockQueueConfig,
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        request_component: TReq,
        block_stream: BlockStream<N>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
    ) -> Self {
        let current_macro_height = policy::last_macro_block(blockchain.read().block_number());
        Self {
            block_stream,
            inner: Inner {
                config,
                blockchain,
                network,
                request_component,
                buffer: BTreeMap::new(),
                push_ops: VecDeque::new(),
                pending_blocks: BTreeSet::new(),
                waker: None,
                current_macro_height,
                bls_cache,
            },
            accepted_announcements: 0,
        }
    }

    /// Returns an iterator over the buffered blocks
    pub fn buffered_blocks(&self) -> impl Iterator<Item = (u32, Vec<&Block>)> {
        self.inner.buffer.iter().map(|(block_number, blocks)| {
            (
                *block_number,
                blocks.values().map(|(block, _pubsub_id)| block).collect(),
            )
        })
    }

    pub fn num_peers(&self) -> usize {
        self.inner.request_component.num_peers()
    }

    pub fn peers(&self) -> Vec<N::PeerId> {
        self.inner.request_component.peers()
    }

    pub fn accepted_block_announcements(&self) -> usize {
        self.accepted_announcements
    }

    pub fn push_block(&mut self, block: Block, peer_id: N::PeerId) {
        self.inner.on_block_announced(block, peer_id, None);
    }

    pub fn request_component(&self) -> &TReq {
        &self.inner.request_component
    }
}

impl<N: Network, TReq: RequestComponent<N>> Stream for BlockQueue<N, TReq> {
    type Item = BlockQueueEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let num_peers = self.num_peers();
        let mut this = self.project();

        // First, advance the internal future to process buffered blocks.
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(BlockQueueEvent::AcceptedAnnouncedBlock(hash))) => {
                *this.accepted_announcements = this.accepted_announcements.saturating_add(1);
                return Poll::Ready(Some(BlockQueueEvent::AcceptedAnnouncedBlock(hash)));
            }
            Poll::Ready(Some(event)) => {
                return Poll::Ready(Some(event));
            }
            Poll::Ready(None) => unreachable!(),
            Poll::Pending => {}
        }

        // Then, try to get as many blocks from the gossipsub stream as possible.
        loop {
            match this.block_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some((block, pubsub_id))) => {
                    // Ignore all block announcements until there is at least one synced peer.
                    if num_peers > 0 {
                        log::debug!(%block, "Received block via gossipsub");
                        this.inner.on_block_announced(
                            block,
                            pubsub_id.propagation_source(),
                            Some(pubsub_id),
                        );
                    }
                }
                // If the block_stream is exhausted, we quit as well.
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => break,
            }
        }

        Poll::Pending
    }
}
