use std::collections::VecDeque;
use std::task::Waker;
use std::{
    collections::{BTreeMap, HashSet},
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context, Poll},
};

use futures::future::BoxFuture;
use futures::stream::{BoxStream, Stream, StreamExt};
use futures::FutureExt;
use pin_project::pin_project;
use tokio::task::spawn_blocking;

use blockchain::AbstractBlockchain;
use network_interface::{
    network::{MsgAcceptance, Network, PubsubId, Topic},
    peer::Peer,
};
use nimiq_block::Block;
use nimiq_blockchain::{Blockchain, PushError, PushResult};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;

use crate::consensus_agent::ConsensusAgent;
use crate::sync::request_component::RequestComponentEvent;

use super::request_component::RequestComponent;

#[derive(Clone, Debug, Default)]
pub struct BlockTopic;

impl Topic for BlockTopic {
    type Item = Block;

    fn topic(&self) -> String {
        "blocks".to_owned()
    }

    fn validate(&self) -> bool {
        true
    }
}

pub type BlockStream<N> = BoxStream<'static, (Block, <N as Network>::PubsubId)>;

#[derive(Clone, Debug)]
pub enum BlockQueueEvent {
    AcceptedAnnouncedBlock,
    ReceivedMissingBlocks,
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
    blockchain: Arc<Blockchain>,

    /// Reference to the network
    network: Arc<N>,

    /// Buffered blocks - `block_height -> [Block]`. There can be multiple blocks at a height if there are forks.
    ///
    /// # TODO
    ///
    ///  - The inner `Vec` should really be a `SmallVec<[Block; 1]>` or similar.
    ///
    buffer: BTreeMap<u32, Vec<Block>>,

    /// Vector of pending `blockchain.push()` operations.
    push_ops: VecDeque<BoxFuture<'static, PushOpResult>>,

    waker: Option<Waker>,
}

enum PushOpResult {
    Single(Result<PushResult, PushError>),
    Multi(Result<PushResult, PushError>, HashSet<Blake2bHash>),
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
        let block_height = block.block_number();
        let head_height = self.blockchain.block_number();

        if block_height <= head_height {
            // Fork block
            self.push_block(block, pubsub_id);
        } else if block_height == head_height + 1 {
            // New head block
            self.push_block(block, pubsub_id);
        } else if block_height > head_height + self.config.window_max {
            log::warn!(
                "Discarding block #{} outside of buffer window (max {}).",
                block_height,
                head_height + self.config.window_max,
            );

            if let Some(peer) = self.network.get_peer(peer_id) {
                request_component.put_peer_into_sync_mode(peer);
            }
        } else if self.buffer.len() >= self.config.buffer_max {
            log::warn!(
                "Discarding block #{}, buffer full (max {})",
                block_height,
                self.buffer.len(),
            )
        } else {
            let block_hash = block.hash();
            let head_hash = self.blockchain.head_hash();

            let prev_macro_block_height = policy::last_macro_block(head_height);

            // Put block inside buffer window
            self.insert_into_buffer(block);

            log::debug!("Requesting missing blocks: target_hash = {}, head_hash = {}, prev_macro_block_height = {}", block_hash, head_hash, prev_macro_block_height);
            log::trace!(
                "n = {} = {} - {} + 1",
                head_height - prev_macro_block_height + 1,
                head_height,
                prev_macro_block_height
            );

            // get block locators
            let block_locators = self
                .blockchain
                .chain_store
                .get_blocks_backward(
                    &head_hash,
                    block_height - prev_macro_block_height + 2,
                    false,
                    None,
                )
                .into_iter()
                .map(|block| block.hash())
                .collect::<Vec<Blake2bHash>>();

            log::trace!("block_locators = {:?}", block_locators);

            request_component.request_missing_blocks(block_hash, block_locators);
        }
    }

    fn on_missing_blocks_received(&mut self, blocks: Vec<Block>) {
        if blocks.is_empty() {
            log::debug!("Received empty missing blocks response");
            return;
        }

        let blockchain = Arc::clone(&self.blockchain);
        let future = async move {
            let mut block_iter = blocks.into_iter();

            // Hashes of invalid blocks
            let mut invalid_blocks = HashSet::new();

            // Initialize push_result to some random value.
            // It is always overwritten in the first loop iteration.
            let mut push_result = Err(PushError::Orphan);

            // Try to push blocks, until we encounter an inferior or invalid block
            #[allow(clippy::while_let_on_iterator)]
            while let Some(block) = block_iter.next() {
                let block_hash = block.hash();

                log::debug!(
                    "Pushing block #{} from missing blocks response",
                    block.block_number()
                );

                let blockchain1 = Arc::clone(&blockchain);
                push_result = spawn_blocking(move || blockchain1.push(block))
                    .await
                    .expect("blockchain.push() should not panic");
                match &push_result {
                    Ok(PushResult::Ignored) => {
                        log::warn!("Inferior chain - Aborting");
                        invalid_blocks.insert(block_hash);
                        break;
                    }
                    Err(e) => {
                        log::warn!("Failed to push missing block: {}", e);
                        invalid_blocks.insert(block_hash);
                        break;
                    }
                    Ok(result) => {
                        log::debug!("Missing block pushed: {:?}", result);
                    }
                }
            }

            // If there are remaining blocks in the iterator, those are invalid.
            block_iter.for_each(|block| {
                invalid_blocks.insert(block.hash());
            });

            PushOpResult::Multi(push_result, invalid_blocks)
        };

        self.push_ops.push_back(future.boxed());
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    /// Pushes the block to the blockchain and returns whether it has extended the blockchain.
    fn push_block(&mut self, block: Block, pubsub_id: Option<<N as Network>::PubsubId>) {
        let blockchain = Arc::clone(&self.blockchain);
        let network = Arc::clone(&self.network);
        let future = async move {
            let push_result = spawn_blocking(move || blockchain.push(block))
                .await
                .expect("blockchain.push() should not panic");
            let acceptance = match &push_result {
                Ok(result) => {
                    log::debug!("Block pushed: {:?}", result);
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
                    Ok(true) => log::trace!("The block message was relayed succesfully"),
                    Ok(false) => log::warn!("Validation took too long: the block message was no longer in the message cache"),
                    Err(e) => log::error!("Network error while relaying block message: {}", e),
                };
            };

            PushOpResult::Single(push_result)
        };

        self.push_ops.push_back(future.boxed());
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    fn push_buffered(&mut self) {
        let head_height = self.blockchain.block_number();
        log::trace!("head_height = {}", head_height);

        // Check if queued block can be pushed to block chain
        if let Some(entry) = self.buffer.first_entry() {
            log::trace!("first entry: {:?}", entry);

            if *entry.key() > head_height + 1 {
                return;
            }

            // Pop block from queue
            let (_, blocks) = entry.remove_entry();

            // If we get a Vec from the BTree, it must not be empty
            assert!(!blocks.is_empty());

            for block in blocks {
                log::trace!(
                    "Pushing block #{} (currently at #{}, {} blocks left)",
                    block.block_number(),
                    head_height,
                    self.buffer.len(),
                );
                self.push_block(block, None);
            }
        }
    }

    fn insert_into_buffer(&mut self, block: Block) {
        self.buffer
            .entry(block.block_number())
            .or_default()
            .push(block)
    }

    fn remove_invalid_blocks(&mut self, mut invalid_blocks: HashSet<Blake2bHash>) {
        if invalid_blocks.is_empty() {
            return;
        }

        log::trace!("Removing any blocks that depend on: {:?}", invalid_blocks);

        // Iterate over all offsets, remove element if no blocks remain at that offset.
        self.buffer.drain_filter(|_block_number, blocks| {
            // Iterate over all blocks at an offset, remove block, if parent is invalid
            blocks.drain_filter(|block| {
                if invalid_blocks.contains(block.parent_hash()) {
                    log::trace!("Removing block because parent is invalid: {}", block.hash());
                    invalid_blocks.insert(block.hash());
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
        // Store waker.
        match &mut self.waker {
            Some(waker) if !waker.will_wake(cx.waker()) => *waker = cx.waker().clone(),
            None => self.waker = Some(cx.waker().clone()),
            _ => {}
        };

        if let Some(op) = self.push_ops.front_mut() {
            let result = ready!(op.poll_unpin(cx));
            self.push_ops.pop_front().expect("PushOp should be present");

            match result {
                PushOpResult::Single(Ok(result))
                    if result == PushResult::Extended || result == PushResult::Rebranched =>
                {
                    self.push_buffered();
                    return Poll::Ready(Some(BlockQueueEvent::AcceptedAnnouncedBlock));
                }
                PushOpResult::Multi(_, invalid_blocks) => {
                    self.remove_invalid_blocks(invalid_blocks);
                    self.push_buffered();
                    return Poll::Ready(Some(BlockQueueEvent::ReceivedMissingBlocks));
                }
                _ => {}
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
        blockchain: Arc<Blockchain>,
        network: Arc<N>,
        request_component: TReq,
    ) -> Self {
        let block_stream = network
            .subscribe::<BlockTopic>(&BlockTopic::default())
            .await
            .unwrap()
            .boxed();

        Self::with_block_stream(config, blockchain, network, request_component, block_stream)
    }

    pub fn with_block_stream(
        config: BlockQueueConfig,
        blockchain: Arc<Blockchain>,
        network: Arc<N>,
        request_component: TReq,
        block_stream: BlockStream<N>,
    ) -> Self {
        let buffer = BTreeMap::new();

        Self {
            request_component,
            block_stream,
            inner: Inner {
                config,
                blockchain,
                network,
                buffer,
                push_ops: VecDeque::new(),
                waker: None,
            },
            accepted_announcements: 0,
        }
    }

    /// Returns an iterator over the buffered blocks
    pub fn buffered_blocks(&self) -> impl Iterator<Item = (u32, &[Block])> {
        self.inner
            .buffer
            .iter()
            .map(|(block_number, blocks)| (*block_number, blocks.as_ref()))
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
        let this = self.project();

        // First, advance the internal future to process buffered blocks.
        match this.inner.poll_next_unpin(cx) {
            Poll::Ready(Some(BlockQueueEvent::AcceptedAnnouncedBlock)) => {
                *this.accepted_announcements = this.accepted_announcements.saturating_add(1);
                return Poll::Ready(Some(BlockQueueEvent::AcceptedAnnouncedBlock));
            }
            Poll::Ready(Some(BlockQueueEvent::ReceivedMissingBlocks)) => {
                return Poll::Ready(Some(BlockQueueEvent::ReceivedMissingBlocks))
            }
            Poll::Ready(None) => unreachable!(),
            Poll::Pending => {}
        }

        // Then, try to get as many blocks from the gossipsub stream as possible.
        match this.block_stream.poll_next(cx) {
            Poll::Ready(Some((block, pubsub_id))) => {
                // Ignore all block announcements until there is at least once synced peer.
                if num_peers > 0 {
                    this.inner.on_block_announced(
                        block,
                        this.request_component,
                        pubsub_id.propagation_source(),
                        Some(pubsub_id),
                    );
                }
                return Poll::Pending;
            }
            // If the block_stream is exhausted, we quit as well.
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        // Then, read all the responses we got for our missing blocks requests.
        match this.request_component.poll_next(cx) {
            Poll::Ready(Some(RequestComponentEvent::ReceivedBlocks(blocks))) => {
                this.inner.on_missing_blocks_received(blocks);
            }
            Poll::Ready(None) => panic!("The request_component stream is exhausted"),
            Poll::Pending => {}
        }

        Poll::Pending
    }
}
