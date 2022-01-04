use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context, Poll, Waker},
};

use futures::future::BoxFuture;
use futures::stream::{BoxStream, Stream, StreamExt};
use futures::FutureExt;
use parking_lot::RwLock;
use pin_project::pin_project;
use tokio::task::spawn_blocking;

use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Direction};
use nimiq_blockchain::{Blockchain, PushError, PushResult};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, PubsubId, Topic},
    peer::Peer,
};
use nimiq_primitives::policy;

use crate::consensus_agent::ConsensusAgent;
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
            buffer_max: 4 * policy::BATCH_LENGTH as usize,
            window_max: 2 * policy::BATCH_LENGTH,
        }
    }
}

struct Inner<N: Network> {
    /// Configuration for the block queue
    config: BlockQueueConfig,

    /// Reference to the block chain
    blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network
    network: Arc<N>,

    /// Buffered blocks - `block_height -> [Block]`. There can be multiple blocks at a height if there are forks.
    ///
    /// # TODO
    ///
    ///  - The inner `Vec` should really be a `SmallVec<[Block; 1]>` or similar.
    ///
    buffer: BTreeMap<u32, HashMap<Blake2bHash, Block>>,

    /// Vector of pending `blockchain.push()` operations.
    push_ops: VecDeque<BoxFuture<'static, PushOpResult>>,

    /// Hashes of blocks that are pending to be pushed to the chain.
    pending_blocks: BTreeSet<Blake2bHash>,

    waker: Option<Waker>,
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

impl<N: Network> Inner<N> {
    /// Handles a block announcement.
    fn on_block_announced<TReq: RequestComponent<N::PeerType>>(
        &mut self,
        block: Block,
        mut request_component: Pin<&mut TReq>,
        peer_id: <N::PeerType as Peer>::Id,
        pubsub_id: Option<<N as Network>::PubsubId>,
    ) {
        let block_number = block.block_number();
        let view_number = block.view_number();

        // Ideally we would acquire upgradable read here. And provide it to push_block.
        // RwLockUpgradableRead however is not Send and cannot be moved into the future that is created.
        // However the push_block does not push the block but creates a future which pushes the block.
        // Hence every future would only be created when an RwLockUpgradableReadGuard can be acquired, requiring
        // to wait for the previous future to have been polled and concluded.
        // Therefore the read here needs to suffice even though the condition (block_height <= head_height) might
        //  no longer be met once the future executes.
        let blockchain = self.blockchain.read();
        let head_height = blockchain.block_number();

        if blockchain.contains(block.parent_hash(), true) {
            // New head or fork block.
            // TODO We should limit the number of push operations we queue here.
            drop(blockchain);
            self.push_block(block, pubsub_id, PushOpResult::Head);
        } else if block_number > head_height + self.config.window_max {
            log::warn!(
                "Discarding block #{}.{} outside of buffer window (max {})",
                block_number,
                view_number,
                head_height + self.config.window_max,
            );

            if let Some(peer) = self.network.get_peer(peer_id) {
                request_component.put_peer_into_sync_mode(peer);
            }
        } else if self.buffer.len() >= self.config.buffer_max {
            log::warn!(
                "Discarding block #{}.{}, buffer full (max {})",
                block_number,
                view_number,
                self.buffer.len(),
            )
        } else {
            // Block is inside the buffer window, put it in the buffer.
            let block_hash = block.hash();
            let parent_hash = block.parent_hash().clone();

            // Insert block into buffer. If we already know the block, we're done.
            let block_known = self
                .buffer
                .entry(block_number)
                .or_default()
                .insert(block_hash.clone(), block)
                .is_some();
            log::trace!(
                "Buffering block #{}.{}, known={}",
                block_number,
                view_number,
                block_known
            );
            if block_known {
                return;
            }

            // If the parent of this block is already in the buffer, we're done.
            let parent_buffered = self
                .buffer
                .get(&(block_number - 1))
                .map_or(false, |blocks| blocks.contains_key(&parent_hash));
            log::trace!(
                "Parent of block #{}.{} buffered={}",
                block_number,
                view_number,
                parent_buffered
            );
            if parent_buffered {
                return;
            }

            // If the parent of this block is already being pushed, we're done.
            let parent_pending = self.pending_blocks.contains(&parent_hash);
            log::trace!(
                "Parent of block #{}.{} pending={}",
                block_number,
                view_number,
                parent_pending
            );
            if parent_pending {
                return;
            }

            // We don't know the predecessor of this block, request it.
            let head_hash = blockchain.head_hash();
            let prev_macro_block_height = policy::last_macro_block(head_height);

            log::debug!("Requesting missing blocks: target_hash = {}, head_hash = {}, prev_macro_block_height = {}", block_hash, head_hash, prev_macro_block_height);

            // Get block locators.
            let block_locators = blockchain
                .chain_store
                .get_blocks(
                    &head_hash,
                    // FIXME We don't want to send the full batch as locators here.
                    block_number - prev_macro_block_height + 2,
                    false,
                    Direction::Backward,
                    None,
                )
                .into_iter()
                .map(|block| block.hash())
                .collect::<Vec<Blake2bHash>>();

            request_component.request_missing_blocks(parent_hash, block_locators);
        }
    }

    fn on_missing_blocks_received(&mut self, blocks: Vec<Block>) {
        if blocks.is_empty() {
            log::debug!("Received empty missing blocks response");
            return;
        }

        self.pending_blocks
            .extend(blocks.iter().map(|block| block.hash()));

        let blockchain = Arc::clone(&self.blockchain);
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

                log::debug!(
                    "Pushing block #{} from missing blocks response",
                    block.block_number()
                );

                let blockchain1 = Arc::clone(&blockchain);
                push_result =
                    spawn_blocking(move || Blockchain::push(blockchain1.upgradable_read(), block))
                        .await
                        .expect("blockchain.push() should not panic");
                match &push_result {
                    Err(e) => {
                        log::warn!("Failed to push missing block: {}", e);
                        invalid_blocks.insert(block_hash);
                        break;
                    }
                    Ok(result) => {
                        log::trace!("Missing block pushed: {:?}", result);
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
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
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
        let network = Arc::clone(&self.network);
        let future = async move {
            let push_result =
                spawn_blocking(move || Blockchain::push(blockchain.upgradable_read(), block))
                    .await
                    .expect("blockchain.push() should not panic");
            let acceptance = match &push_result {
                Ok(result) => {
                    log::trace!("Block pushed: {:?}", result);
                    match result {
                        PushResult::Known | PushResult::Extended | PushResult::Rebranched => {
                            MsgAcceptance::Accept
                        }
                        PushResult::Forked | PushResult::Ignored => MsgAcceptance::Ignore,
                    }
                }
                Err(e) => {
                    log::warn!("Failed to push block: {}", e);
                    // TODO Ban peer
                    MsgAcceptance::Reject
                }
            };

            // Let the network layer know if it should relay the message this block came from
            if let Some(pubsub_id) = pubsub_id {
                match network.validate_message(pubsub_id, acceptance).await {
                    Ok(true) => {}, // success
                    Ok(false) => log::warn!("Validation took too long: the block message was no longer in the message cache"),
                    Err(e) => log::error!("Network error while relaying block message: {}", e),
                };
            };

            op(push_result, block_hash)
        };

        // TODO We should limit the number of push operations we queue here.
        self.push_ops.push_back(future.boxed());
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    fn push_buffered(&mut self) {
        let mut blocks_to_push = vec![];
        {
            let blockchain = self.blockchain.read();
            self.buffer.drain_filter(|_, blocks| {
                // Push all blocks with a known parent to the chain.
                let blocks_with_known_parent = blocks
                    .drain_filter(|_, block| blockchain.contains(block.parent_hash(), true))
                    .map(|(_, block)| block);
                blocks_to_push.extend(blocks_with_known_parent);

                // Remove buffer entry if there are no blocks left.
                blocks.is_empty()
            });
        }

        for block in blocks_to_push {
            self.push_block(block, None, PushOpResult::Buffered);
        }
    }

    fn remove_invalid_blocks(&mut self, mut invalid_blocks: HashSet<Blake2bHash>) {
        if invalid_blocks.is_empty() {
            return;
        }

        // Iterate over all offsets, remove element if no blocks remain at that offset.
        self.buffer.drain_filter(|_block_number, blocks| {
            // Iterate over all blocks at an offset, remove block, if parent is invalid
            blocks.drain_filter(|hash, block| {
                if invalid_blocks.contains(block.parent_hash()) {
                    log::trace!("Removing block because parent is invalid: {}", hash);
                    invalid_blocks.insert(hash.clone());
                    true
                } else {
                    false
                }
            });
            blocks.is_empty()
        });
    }
}

impl<N: Network> Stream for Inner<N> {
    type Item = BlockQueueEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        store_waker!(self, waker, cx);

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
                    //If there was a blockchain push error, we remove the block from the pending blocks
                    log::trace!("Head push operation failed because of {}", result);
                    self.pending_blocks.remove(&hash);
                    return Poll::Ready(Some(BlockQueueEvent::RejectedBlock(hash)));
                }

                PushOpResult::Buffered(Err(result), hash) => {
                    //If there was a blockchain push error, we remove the block from the pending blocks
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
pub struct BlockQueue<N: Network, TReq: RequestComponent<N::PeerType>> {
    /// The Peer Tracking and Request Component.
    #[pin]
    pub request_component: TReq,

    /// The blocks received via gossipsub.
    #[pin]
    block_stream: BlockStream<N>,

    /// The inner state of the block queue.
    inner: Inner<N>,

    /// The number of extended blocks through announcements.
    accepted_announcements: usize,
}

impl<N: Network, TReq: RequestComponent<N::PeerType>> BlockQueue<N, TReq> {
    pub async fn new(
        config: BlockQueueConfig,
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        request_component: TReq,
    ) -> Self {
        let block_stream = network.subscribe::<BlockTopic>().await.unwrap().boxed();

        Self::with_block_stream(config, blockchain, network, request_component, block_stream)
    }

    pub fn with_block_stream(
        config: BlockQueueConfig,
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        request_component: TReq,
        block_stream: BlockStream<N>,
    ) -> Self {
        Self {
            request_component,
            block_stream,
            inner: Inner {
                config,
                blockchain,
                network,
                buffer: BTreeMap::new(),
                push_ops: VecDeque::new(),
                pending_blocks: BTreeSet::new(),
                waker: None,
            },
            accepted_announcements: 0,
        }
    }

    /// Returns an iterator over the buffered blocks
    pub fn buffered_blocks(&self) -> impl Iterator<Item = (u32, Vec<&Block>)> {
        self.inner
            .buffer
            .iter()
            .map(|(block_number, blocks)| (*block_number, blocks.values().collect()))
    }

    pub fn num_peers(&self) -> usize {
        self.request_component.num_peers()
    }

    pub fn peers(&self) -> Vec<Weak<ConsensusAgent<N::PeerType>>> {
        self.request_component.peers()
    }

    pub fn accepted_block_announcements(&self) -> usize {
        self.accepted_announcements
    }

    pub fn push_block(&mut self, block: Block, peer_id: <N::PeerType as Peer>::Id) {
        self.inner
            .on_block_announced(block, Pin::new(&mut self.request_component), peer_id, None);
    }
}

impl<N: Network, TReq: RequestComponent<N::PeerType>> Stream for BlockQueue<N, TReq> {
    type Item = BlockQueueEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let num_peers = self.num_peers();
        let mut this = self.project();

        // First, advance the internal future to process buffered blocks.
        match this.inner.poll_next_unpin(cx) {
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
                        log::debug!(
                            "Received block #{}.{} via gossipsub",
                            block.block_number(),
                            block.view_number()
                        );
                        this.inner.on_block_announced(
                            block,
                            this.request_component.as_mut(),
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

        // Then, read all the responses we got for our missing blocks requests.
        loop {
            match this.request_component.as_mut().poll_next(cx) {
                Poll::Ready(Some(RequestComponentEvent::ReceivedBlocks(blocks))) => {
                    this.inner.on_missing_blocks_received(blocks);
                }
                Poll::Ready(None) => unreachable!(),
                Poll::Pending => break,
            }
        }

        Poll::Pending
    }
}
