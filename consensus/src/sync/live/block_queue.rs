use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::stream::{empty, select, BoxStream};
use futures::{Stream, StreamExt};

use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, Direction};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::{MsgAcceptance, Network, PubsubId, Topic};
use nimiq_primitives::policy::Policy;

use super::{
    block_request_component::{BlockRequestComponentEvent, RequestComponent},
    BlockQueueConfig,
};

#[derive(Clone, Debug, Default)]
pub struct BlockTopic;

impl Topic for BlockTopic {
    type Item = Block;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "blocks";
    const VALIDATE: bool = true;
}

#[derive(Clone, Debug, Default)]
pub struct BlockHeaderTopic;

impl Topic for BlockHeaderTopic {
    type Item = Block;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "block-headers";
    const VALIDATE: bool = true;
}

pub type BlockStream<N> = BoxStream<
    'static,
    (
        Block,
        <N as Network>::PeerId,
        Option<<N as Network>::PubsubId>,
    ),
>;
pub type GossipSubBlockStream<N> = BoxStream<'static, (Block, <N as Network>::PubsubId)>;

type BlockAndId<N> = (Block, Option<<N as Network>::PubsubId>);

pub enum QueuedBlock<N: Network> {
    Head(BlockAndId<N>),
    Buffered(Vec<BlockAndId<N>>),
    Missing(Vec<Block>),
    TooFarFuture(Block, N::PeerId),
    TooDistantPast(Block, N::PeerId),
}

pub struct BlockQueue<N: Network, TReq: RequestComponent<N>> {
    /// Configuration for the block queue
    config: BlockQueueConfig,

    /// Reference to the blockchain
    blockchain: BlockchainProxy,

    /// Reference to the network
    network: Arc<N>,

    /// The Peer Tracking and Request Component.
    request_component: TReq,

    /// A stream of blocks.
    /// This includes blocks received via gossipsub, but can also include other sources.
    block_stream: BlockStream<N>,

    /// Buffered blocks - `block_height -> block_hash -> BlockAndId`.
    /// There can be multiple blocks at a height if there are forks.
    pub(crate) buffer: BTreeMap<u32, HashMap<Blake2bHash, BlockAndId<N>>>,

    /// Hashes of blocks that are pending to be pushed to the chain.
    blocks_pending_push: BTreeSet<Blake2bHash>,

    /// The blockchain event stream.
    blockchain_rx: BoxStream<'static, BlockchainEvent>,

    /// The block number of the latest macro block. We prune the block buffer when it changes.
    current_macro_height: u32,
}

impl<N: Network, TReq: RequestComponent<N>> BlockQueue<N, TReq> {
    pub async fn new(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        request_component: TReq,
        config: BlockQueueConfig,
    ) -> Self {
        let block_stream = if config.include_micro_bodies {
            network.subscribe::<BlockTopic>().await.unwrap().boxed()
        } else {
            network
                .subscribe::<BlockHeaderTopic>()
                .await
                .unwrap()
                .boxed()
        };

        Self::with_gossipsub_block_stream(
            blockchain,
            network,
            request_component,
            block_stream,
            config,
        )
    }

    pub fn with_gossipsub_block_stream(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        request_component: TReq,
        block_stream: GossipSubBlockStream<N>,
        config: BlockQueueConfig,
    ) -> Self {
        let block_stream = block_stream
            .map(|(block, pubsub_id)| (block, pubsub_id.propagation_source(), Some(pubsub_id)))
            .boxed();
        Self::with_block_stream(blockchain, network, request_component, block_stream, config)
    }

    pub fn with_block_stream(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        request_component: TReq,
        block_stream: BlockStream<N>,
        config: BlockQueueConfig,
    ) -> Self {
        let current_macro_height = Policy::last_macro_block(blockchain.read().block_number());
        let blockchain_rx = blockchain.read().notifier_as_stream();
        Self {
            config,
            blockchain,
            blockchain_rx,
            network,
            request_component,
            block_stream,
            buffer: BTreeMap::new(),
            blocks_pending_push: BTreeSet::new(),
            current_macro_height,
        }
    }

    /// Adds an additional block stream by replacing the current block stream with a `select` of both streams.
    pub fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = (Block, N::PeerId, Option<N::PubsubId>)> + Send + 'static,
    {
        // We need to safely remove the old block stream first.
        let prev_block_stream = mem::replace(&mut self.block_stream, empty().boxed());
        self.block_stream = select(prev_block_stream, block_stream).boxed();
    }

    /// Returns an iterator over the buffered blocks
    pub fn buffered_blocks(&self) -> impl Iterator<Item = (u32, Vec<&Block>)> {
        self.buffer.iter().map(|(block_number, blocks)| {
            (
                *block_number,
                blocks.values().map(|(block, _pubsub_id)| block).collect(),
            )
        })
    }

    pub fn buffered_blocks_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn on_block_processed(&mut self, block_hash: &Blake2bHash) {
        self.blocks_pending_push.remove(block_hash);
    }

    pub fn num_peers(&self) -> usize {
        self.request_component.num_peers()
    }

    pub fn include_micro_bodies(&self) -> bool {
        self.config.include_micro_bodies
    }

    pub fn peers(&self) -> Vec<N::PeerId> {
        self.request_component.peers()
    }

    pub fn add_peer(&self, peer_id: N::PeerId) {
        self.request_component.add_peer(peer_id)
    }

    /// Handles a block announcement.
    fn check_announced_block(
        &mut self,
        block: Block,
        peer_id: N::PeerId,
        pubsub_id: Option<<N as Network>::PubsubId>,
    ) -> Option<QueuedBlock<N>> {
        let blockchain = self.blockchain.read();

        let block_number = block.block_number();
        let head_height = blockchain.block_number();

        // Ignore blocks that we already know.
        if blockchain.contains(&block.hash(), true) {
            return None;
        }

        let parent_known = blockchain.contains(block.parent_hash(), true);
        drop(blockchain);

        // Check if a macro block boundary was passed. If so prune the block buffer.
        let macro_height = Policy::last_macro_block(head_height);
        if macro_height > self.current_macro_height {
            self.current_macro_height = macro_height;
            self.prune_buffer();
        }

        if block_number < head_height.saturating_sub(self.config.tolerate_past_max) {
            log::warn!(
                "Discarding block {} earlier than toleration window (max {})",
                block,
                head_height - self.config.tolerate_past_max,
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);

            if self.network.has_peer(peer_id) {
                self.request_component.take_peer(&peer_id);
                return Some(QueuedBlock::TooDistantPast(block, peer_id));
            }
        } else if parent_known {
            // New head or fork block.
            // Add block to pending blocks and return queued block for the stream.
            if self.blocks_pending_push.insert(block.hash()) {
                return Some(QueuedBlock::Head((block, pubsub_id)));
            }
        } else if block_number > head_height + self.config.window_ahead_max {
            log::warn!(
                "Discarding block {} outside of buffer window (max {})",
                block,
                head_height + self.config.window_ahead_max,
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);

            if self.network.has_peer(peer_id) {
                self.request_component.take_peer(&peer_id);
                return Some(QueuedBlock::TooFarFuture(block, peer_id));
            }
        } else if self.buffer.len() >= self.config.buffer_max {
            // TODO: This does not account for the nested map
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

        None
    }

    /// Buffers the current block and requests any missing blocks in-between.
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
        let parent_pending = self.blocks_pending_push.contains(&parent_hash);
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

    /// Requests missing blocks.
    fn request_missing_blocks(&mut self, block_number: u32, block_hash: Blake2bHash) {
        let blockchain = self.blockchain.read();
        let head_hash = blockchain.head_hash();
        let head_height = blockchain.block_number();
        let macro_height = Policy::last_macro_block(head_height);

        log::debug!(
            block_number,
            %block_hash,
            %head_hash,
            macro_height,
            "Requesting missing blocks",
        );

        // Get block locators. The blocks returned by `get_blocks` do *not* include the start block.
        // FIXME We don't want to send the full batch as locators here.
        let blocks = blockchain.get_blocks(
            &head_hash,
            head_height - macro_height,
            false,
            Direction::Backward,
        );
        if let Ok(blocks) = blocks {
            let block_locators = blocks.into_iter().map(|block| block.hash());

            // Prepend our current head hash.
            let block_locators = vec![head_hash].into_iter().chain(block_locators).collect();

            // FIXME Send missing blocks request to the peer that announced the block (first).
            self.request_component
                .request_missing_blocks(block_hash, block_locators);
        } else {
            log::error!(start_block = %head_hash, count = head_height - macro_height, "Couldn't get blocks")
        }
    }

    /// Handles missing blocks that were received.
    fn handle_missing_blocks(&mut self, blocks: Vec<Block>) -> Option<QueuedBlock<N>> {
        if blocks.is_empty() {
            log::debug!("Received empty missing blocks response");
            return None;
        }

        // FIXME Sanity-check blocks

        // Check whether the blockchain can push the missing blocks. This might not be the case if the reference
        // block used in the request is from a batch that we have not adopted yet.
        let parent_known = self
            .blockchain
            .read()
            .contains(blocks[0].parent_hash(), true);
        if !parent_known {
            // The blockchain cannot process the blocks right away, put them in the buffer.
            // Recursively request missing blocks for the first block we received.
            let mut blocks = blocks.into_iter();
            let first_block = blocks.next().unwrap();
            self.buffer_and_request_missing_blocks(first_block, None);

            // Store the remaining blocks in the buffer.
            for block in blocks {
                self.insert_block_into_buffer(block, None);
            }

            return None;
        }

        // Return missing blocks so they can be pushed to the chain.
        self.blocks_pending_push
            .extend(blocks.iter().map(|block| block.hash()));

        Some(QueuedBlock::Missing(blocks))
    }

    /// Removes and returns all buffered blocks whose parent is known to the blockchain.
    fn remove_applicable_blocks(&mut self, event: BlockchainEvent) -> Vec<BlockAndId<N>> {
        // Collect block numbers and hashes of newly added blocks first.
        let mut block_infos = vec![];
        match event {
            BlockchainEvent::Extended(block_hash)
            | BlockchainEvent::HistoryAdopted(block_hash)
            | BlockchainEvent::Finalized(block_hash)
            | BlockchainEvent::EpochFinalized(block_hash) => {
                if let Ok(block) = self.blockchain.read().get_block(&block_hash, false) {
                    block_infos.push((block.block_number(), block_hash));
                }
            }
            BlockchainEvent::Rebranched(_, new_blocks) => {
                for (block_hash, block) in new_blocks {
                    block_infos.push((block.block_number(), block_hash));
                }
            }
        }

        // The only blocks that can now be applied but couldn't be before
        // are those whose parent is one of the newly added blocks.
        // So, we specifically collect and remove those.
        let mut blocks_to_push = vec![];
        for (new_block_number, new_block_hash) in block_infos {
            // Get the blocks following the newly added block.
            let mut is_empty = false;
            if let Some(blocks) = self.buffer.get_mut(&(new_block_number + 1)) {
                // Collect all blocks with a known parent.
                blocks.retain(|_, (block, pubsub_id)| {
                    let push = block.parent_hash() == &new_block_hash;
                    if push {
                        blocks_to_push.push((block.clone(), pubsub_id.clone()));
                    }
                    !push
                });
                is_empty = blocks.is_empty();
            }
            // Clean up empty maps.
            if is_empty {
                self.buffer.remove(&(new_block_number + 1));
            }
        }

        // Remove blocks that we have returned previously but that are not processed by the blockchain yet.
        blocks_to_push.retain(|(block, _)| self.blocks_pending_push.insert(block.hash()));

        blocks_to_push
    }

    /// Removes a set of invalid blocks and all dependent blocks from the buffer.
    pub fn remove_invalid_blocks(&mut self, invalid_blocks: &mut HashSet<Blake2bHash>) {
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
                        if self.config.include_micro_bodies {
                            self.network
                                .validate_message::<BlockTopic>(id.clone(), MsgAcceptance::Reject);
                        } else {
                            self.network.validate_message::<BlockHeaderTopic>(
                                id.clone(),
                                MsgAcceptance::Reject,
                            );
                        }
                    }

                    false
                } else {
                    true
                }
            });
            !blocks.is_empty()
        });
    }

    /// Cleans up buffered blocks and removes blocks that precede the current macro block.
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
                    if self.config.include_micro_bodies {
                        self.network
                            .validate_message::<BlockTopic>(id.clone(), MsgAcceptance::Ignore);
                    } else {
                        self.network.validate_message::<BlockHeaderTopic>(
                            id.clone(),
                            MsgAcceptance::Ignore,
                        );
                    }
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
            if self.config.include_micro_bodies {
                self.network.validate_message::<BlockTopic>(id, acceptance);
            } else {
                self.network
                    .validate_message::<BlockHeaderTopic>(id, acceptance);
            }
        }
    }
}

impl<N: Network, TReq: RequestComponent<N>> Stream for BlockQueue<N, TReq> {
    type Item = QueuedBlock<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll the blockchain stream and return blocks that can now possibly be pushed by the blockchain.
        while let Poll::Ready(Some(event)) = self.blockchain_rx.poll_next_unpin(cx) {
            let buffered_blocks = self.remove_applicable_blocks(event);
            if !buffered_blocks.is_empty() {
                return Poll::Ready(Some(QueuedBlock::Buffered(buffered_blocks)));
            }
        }

        // Get as many blocks from the gossipsub stream as possible.
        loop {
            match self.block_stream.poll_next_unpin(cx) {
                Poll::Ready(Some((block, peer_id, pubsub_id))) => {
                    // Ignore all block announcements until there is at least one synced peer.
                    if self.num_peers() > 0 {
                        log::debug!(%block, "Received block via gossipsub");
                        if let Some(block) = self.check_announced_block(block, peer_id, pubsub_id) {
                            return Poll::Ready(Some(block));
                        }
                    }
                }
                // If the block_stream is exhausted, we quit as well.
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => break,
            }
        }

        // Read all the responses we got for our missing blocks requests.
        loop {
            let poll_res = self.request_component.poll_next_unpin(cx);
            match poll_res {
                Poll::Ready(Some(BlockRequestComponentEvent::ReceivedBlocks(blocks))) => {
                    if let Some(block) = self.handle_missing_blocks(blocks) {
                        return Poll::Ready(Some(block));
                    }
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => break,
            }
        }

        Poll::Pending
    }
}
