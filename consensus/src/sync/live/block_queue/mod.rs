use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{stream::BoxStream, Stream, StreamExt};
use nimiq_block::{Block, BlockHeaderTopic, BlockTopic};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, Direction, ForkEvent};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::{MsgAcceptance, Network, PubsubId};
use nimiq_primitives::policy::Policy;
use parking_lot::RwLock;

use self::block_request_component::BlockRequestComponent;
use super::{block_queue::block_request_component::BlockRequestComponentEvent, queue::QueueConfig};
use crate::sync::peer_list::PeerList;

pub mod block_request_component;
pub mod live_sync;

pub type BlockStream<N> = BoxStream<
    'static,
    (
        Block,
        <N as Network>::PeerId,
        Option<<N as Network>::PubsubId>,
    ),
>;
pub type GossipSubBlockStream<N> = BoxStream<'static, (Block, <N as Network>::PubsubId)>;

pub type BlockAndId<N> = (Block, Option<<N as Network>::PubsubId>);

pub enum QueuedBlock<N: Network> {
    Head(BlockAndId<N>),
    Buffered(Vec<BlockAndId<N>>),
    Missing(Vec<Block>),
    TooFarAhead(Block, N::PeerId),
    TooFarBehind(Block, N::PeerId),
}

pub struct BlockQueue<N: Network> {
    /// Configuration for the block queue
    config: QueueConfig,

    /// Reference to the blockchain
    blockchain: BlockchainProxy,

    /// Reference to the network
    network: Arc<N>,

    /// The Peer Tracking and Request Component.
    request_component: BlockRequestComponent<N>,

    /// A stream of blocks.
    /// This includes blocks received via gossipsub, but can also include other sources.
    block_stream: BlockStream<N>,

    /// Buffered blocks - `block_height -> block_hash -> BlockAndId`.
    /// There can be multiple blocks at a height if there are forks.
    buffer: BTreeMap<u32, HashMap<Blake2bHash, BlockAndId<N>>>,

    /// Hashes of blocks that are pending to be pushed to the chain.
    blocks_pending_push: BTreeSet<Blake2bHash>,

    /// The blockchain event stream.
    blockchain_rx: BoxStream<'static, BlockchainEvent>,

    /// The blockchain event stream.
    fork_rx: BoxStream<'static, ForkEvent>,

    /// The block number of the latest macro block. We prune the block buffer when it changes.
    current_macro_height: u32,
}

impl<N: Network> BlockQueue<N> {
    pub async fn new(network: Arc<N>, blockchain: BlockchainProxy, config: QueueConfig) -> Self {
        let block_stream = if config.include_micro_bodies {
            network.subscribe::<BlockTopic>().await.unwrap().boxed()
        } else {
            network
                .subscribe::<BlockHeaderTopic>()
                .await
                .unwrap()
                .boxed()
        };

        Self::with_gossipsub_block_stream(blockchain, network, block_stream, config)
    }

    pub fn with_gossipsub_block_stream(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        block_stream: GossipSubBlockStream<N>,
        config: QueueConfig,
    ) -> Self {
        let block_stream = block_stream
            .map(|(block, pubsub_id)| (block, pubsub_id.propagation_source(), Some(pubsub_id)))
            .boxed();
        Self::with_block_stream(blockchain, network, block_stream, config)
    }

    pub fn with_block_stream(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        block_stream: BlockStream<N>,
        config: QueueConfig,
    ) -> Self {
        let current_macro_height = Policy::last_macro_block(blockchain.read().block_number());
        let blockchain_rx = blockchain.read().notifier_as_stream();
        let fork_rx = blockchain.read().fork_notifier_as_stream();
        let request_component =
            BlockRequestComponent::new(Arc::clone(&network), config.include_micro_bodies);

        Self {
            config,
            blockchain,
            blockchain_rx,
            fork_rx,
            network,
            request_component,
            block_stream,
            buffer: BTreeMap::new(),
            blocks_pending_push: BTreeSet::new(),
            current_macro_height,
        }
    }

    pub fn on_block_processed(&mut self, block_hash: &Blake2bHash) {
        self.blocks_pending_push.remove(block_hash);
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
                "Discarding block {} - too far behind (min {})",
                block,
                head_height - self.config.tolerate_past_max,
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);

            if self.request_component.take_peer(&peer_id).is_some() {
                return Some(QueuedBlock::TooFarBehind(block, peer_id));
            }
        } else if parent_known {
            // New head or fork block.
            // Add block to pending blocks and return queued block for the stream.
            if self.blocks_pending_push.insert(block.hash()) {
                return Some(QueuedBlock::Head((block, pubsub_id)));
            }
        } else if block_number > head_height + self.config.window_ahead_max {
            log::warn!(
                "Discarding block {} - too far ahead (max {})",
                block,
                head_height + self.config.window_ahead_max,
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);

            if self.request_component.take_peer(&peer_id).is_some() {
                return Some(QueuedBlock::TooFarAhead(block, peer_id));
            }
        } else if self.buffer.len() >= self.config.buffer_max {
            // TODO: This does not account for the nested map
            log::warn!(
                "Discarding block {} - buffer full (max {})",
                block,
                self.buffer.len(),
            );
            self.report_validation_result(pubsub_id, MsgAcceptance::Ignore);
        } else if block_number <= macro_height {
            // Block is from a previous batch/epoch, discard it.
            log::warn!(
                "Discarding block {} - we're already at macro block #{}",
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

        let mut parent_hash = block.parent_hash().clone();
        let mut parent_block_number = block_number - 1;

        // Insert block into buffer. If we already know the block, we're done.
        let block_known = self.insert_block_into_buffer(block, pubsub_id.clone());
        log::trace!("Buffering block #{}, known={}", block_number, block_known);
        if block_known {
            return;
        }

        // If the parent of this block is already in the buffer, follow the chain to see whether there are still blocks missing.
        while let Some(parent_block_hash) =
            self.get_parent_from_buffer(parent_block_number, &parent_hash)
        {
            parent_hash = parent_block_hash;
            parent_block_number = parent_block_number.saturating_sub(1);
        }

        // If the parent of this block is already being pushed or we already requested missing blocks for it, we're done.
        let parent_pending = self.blocks_pending_push.contains(&parent_hash)
            || self.request_component.is_pending(&parent_hash);
        log::trace!(
            "Parent of block #{} pending={}",
            block_number,
            parent_pending
        );
        if parent_pending {
            return;
        }

        // We don't know the predecessor of this block, request it.
        self.request_missing_blocks(parent_block_number, parent_hash, None, pubsub_id);
    }

    fn insert_block_into_buffer(&mut self, block: Block, pubsub_id: Option<N::PubsubId>) -> bool {
        self.buffer
            .entry(block.block_number())
            .or_default()
            .insert(block.hash(), (block, pubsub_id))
            .is_some()
    }

    fn get_parent_from_buffer(&self, block_number: u32, hash: &Blake2bHash) -> Option<Blake2bHash> {
        self.buffer.get(&block_number).and_then(|blocks| {
            blocks
                .get(hash)
                .map(|(block, _)| block.parent_hash().clone())
        })
    }

    /// Requests missing blocks.
    /// If a block locator is given, it is inserted at the beginning of the full block locator list.
    /// The list contains all blocks from the head until the last macro block.
    fn request_missing_blocks(
        &mut self,
        block_number: u32,
        block_hash: Blake2bHash,
        block_locator: Option<Blake2bHash>,
        pubsub_id: Option<N::PubsubId>,
    ) {
        let (head_hash, head_height, macro_height, blocks) = {
            let blockchain = self.blockchain.read();
            let head_hash = blockchain.head_hash();
            let head_height = blockchain.block_number();
            let macro_height = Policy::last_macro_block(head_height);

            // Get block locators. The blocks returned by `get_blocks` do *not* include the start block.
            // FIXME We don't want to send the full batch as locators here.
            let blocks = blockchain.get_blocks(
                &head_hash,
                head_height - macro_height,
                false,
                Direction::Backward,
            );
            (head_hash, head_height, macro_height, blocks)
        };

        if let Ok(blocks) = blocks {
            log::debug!(
                block_number,
                %block_hash,
                %head_hash,
                macro_height,
                "Requesting missing blocks",
            );

            let block_locators = blocks.into_iter().map(|block| block.hash());

            let init_block_locators = if let Some(block_locator) = block_locator {
                vec![block_locator, head_hash]
            } else {
                vec![head_hash]
            };
            // Prepend our current head hash.
            let block_locators = init_block_locators
                .into_iter()
                .chain(block_locators)
                .collect();

            self.request_component.request_missing_blocks(
                block_number,
                block_hash,
                block_locators,
                pubsub_id,
            );
        } else {
            log::error!(start_block = %head_hash, count = head_height - macro_height, "Couldn't get blocks")
        }
    }

    /// Handles missing blocks that were received.
    fn handle_missing_blocks(
        &mut self,
        target_block_number: u32,
        target_hash: Blake2bHash,
        blocks: Vec<Block>,
    ) -> Option<QueuedBlock<N>> {
        if blocks.is_empty() {
            log::debug!("Received empty missing blocks response");
            return None;
        }

        // Checks that the first block was part of the chain or block locators.
        let first_block = blocks.first()?;
        if !self
            .blockchain
            .read()
            .contains(first_block.parent_hash(), false)
            && !self.blocks_pending_push.contains(first_block.parent_hash())
            && !self
                .buffer
                .get(&first_block.block_number().saturating_sub(1))
                .map(|blocks| blocks.contains_key(first_block.parent_hash()))
                .unwrap_or(false)
        {
            log::error!("Received invalid chain of missing blocks (first block not in chain or block locators)");
            return None;
        }
        if first_block.block_number() > target_block_number {
            log::error!(
                first_block = first_block.block_number(),
                target_block_number,
                "Received invalid chain of missing blocks (first block > target)"
            );
            return None;
        }

        // Check that the chain of missing blocks is valid.
        let mut previous = first_block;
        for block in blocks.iter().skip(1) {
            if block.block_number() == previous.block_number() + 1
                && block.block_number() <= target_block_number
                && block.parent_hash() == &previous.hash()
            {
                previous = block;
            } else {
                log::error!("Received invalid chain of missing blocks");
                return None;
            }
        }

        let last_block = blocks.last()?;
        let block_hash = last_block.hash();
        if last_block.block_number() == target_block_number && block_hash != target_hash {
            log::error!(
                target_block_number,
                %block_hash,
                %target_hash,
                "Received invalid missing blocks (invalid target block)"
            );
            return None;
        }
        if block_hash != target_hash {
            self.request_missing_blocks(target_block_number, target_hash, Some(block_hash), None);
        }

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

    /// Fetches the block information for the new blocks.
    fn get_new_blocks_from_blockchain_event(
        &self,
        event: BlockchainEvent,
    ) -> Vec<(u32, Blake2bHash)> {
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
        block_infos
    }

    /// Fetches the block information for the new blocks.
    fn get_new_blocks_from_fork_event(&self, event: ForkEvent) -> Vec<(u32, Blake2bHash)> {
        // Collect block numbers and hashes of newly added blocks first.
        let mut block_infos = vec![];

        match event {
            ForkEvent::Detected(proof) => {
                block_infos.push((proof.block_number(), proof.header1_hash()));
                block_infos.push((proof.block_number(), proof.header2_hash()));
            }
        }
        block_infos
    }

    /// Removes and returns all buffered blocks whose parent is known to the blockchain.
    fn remove_applicable_blocks(
        &mut self,
        block_infos: Vec<(u32, Blake2bHash)>,
    ) -> Vec<BlockAndId<N>> {
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

    /// Returns an iterator over the buffered blocks
    pub fn buffered_blocks(&self) -> impl Iterator<Item = (u32, Vec<&Block>)> {
        self.buffer.iter().map(|(block_number, blocks)| {
            (
                *block_number,
                blocks.values().map(|(block, _pubsub_id)| block).collect(),
            )
        })
    }

    /// Returns the number of buffered blocks.
    pub(crate) fn num_buffered_blocks(&self) -> usize {
        self.buffer.len()
    }

    /// Returns the list of peers tracked by this component.
    pub(crate) fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        self.request_component.peer_list()
    }
}

impl<N: Network> Stream for BlockQueue<N> {
    type Item = QueuedBlock<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll the blockchain stream and return blocks that can now possibly be pushed by the blockchain.
        while let Poll::Ready(Some(event)) = self.blockchain_rx.poll_next_unpin(cx) {
            let block_infos = self.get_new_blocks_from_blockchain_event(event);
            let buffered_blocks = self.remove_applicable_blocks(block_infos);
            if !buffered_blocks.is_empty() {
                return Poll::Ready(Some(QueuedBlock::Buffered(buffered_blocks)));
            }
        }

        // Poll the fork stream and return blocks that can now possibly be pushed by the blockchain.
        while let Poll::Ready(Some(event)) = self.fork_rx.poll_next_unpin(cx) {
            let block_infos = self.get_new_blocks_from_fork_event(event);
            let buffered_blocks = self.remove_applicable_blocks(block_infos);
            if !buffered_blocks.is_empty() {
                return Poll::Ready(Some(QueuedBlock::Buffered(buffered_blocks)));
            }
        }

        // Get as many blocks from the gossipsub stream as possible.
        loop {
            match self.block_stream.poll_next_unpin(cx) {
                Poll::Ready(Some((block, peer_id, pubsub_id))) => {
                    // Only consider announcements from synced peers.
                    if self.peer_list().read().has_peer(&peer_id) {
                        log::debug!(%block, %peer_id, "Received block via gossipsub");
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
                Poll::Ready(Some(BlockRequestComponentEvent::ReceivedBlocks(
                    target_block_number,
                    target_hash,
                    blocks,
                ))) => {
                    if let Some(block) =
                        self.handle_missing_blocks(target_block_number, target_hash, blocks)
                    {
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
