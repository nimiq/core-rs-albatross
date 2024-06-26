use std::{
    collections::{
        btree_map::{BTreeMap, Entry as BTreeMapEntry},
        hash_map::{Entry as HashMapEntry, HashMap},
        BTreeSet, HashSet,
    },
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{stream::BoxStream, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, Direction, ForkEvent};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::{MsgAcceptance, Network, PubsubId};
use nimiq_primitives::{policy::Policy, slots_allocation::Validators};
use nimiq_utils::WakerExt;
use parking_lot::RwLock;
use tokio::sync::oneshot::Sender as OneshotSender;

use crate::{
    consensus::{ResolveBlockError, ResolveBlockRequest},
    messages::{BlockBodyTopic, BlockHeaderTopic},
    sync::{
        live::{
            block_queue::{
                assembler::BlockAssembler,
                block_request_component::{BlockRequestComponent, BlockRequestComponentEvent},
                BlockAndId, BlockStream, GossipSubBlockStream, QueuedBlock,
            },
            queue::QueueConfig,
        },
        peer_list::PeerList,
    },
};

pub struct BlockQueue<N: Network> {
    /// Configuration for the block queue
    pub(crate) config: QueueConfig,

    /// Reference to the blockchain
    blockchain: BlockchainProxy,

    /// Reference to the network
    network: Arc<N>,

    /// The Peer Tracking and Request Component.
    pub(crate) request_component: BlockRequestComponent<N>,

    /// A stream of blocks.
    /// This includes blocks received via gossipsub, but can also include other sources.
    pub(crate) block_stream: BlockStream<N>,

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

    /// A list of all pending missing block requests which have someplace waiting for it to resolve.
    ///
    /// `block_height` -> `block_hash` -> `OneshotSender` to resolve them.
    ///
    /// Generally this would be empty as most missing block requests do not have another party waiting
    /// for them to resolve. Currently the only other part waiting for a resolution of such a request is the
    /// ProposalBuffer in the validator crate. It uses it to request predecessors of proposals if they
    /// are unknown.
    ///
    pending_requests:
        BTreeMap<u32, HashMap<Blake2bHash, OneshotSender<Result<Block, ResolveBlockError<N>>>>>,

    /// Waker used for the poll function
    pub(crate) waker: Option<Waker>,
}

impl<N: Network> BlockQueue<N> {
    const MAX_BUFFERED_PER_PEER_PER_HEIGHT: usize = 5;

    pub async fn new(network: Arc<N>, blockchain: BlockchainProxy, config: QueueConfig) -> Self {
        let header_stream = network.subscribe::<BlockHeaderTopic>().await.unwrap();

        let block_stream = if config.include_micro_bodies {
            let body_stream = network.subscribe::<BlockBodyTopic>().await.unwrap().boxed();
            BlockAssembler::<N>::new(header_stream, body_stream).boxed()
        } else {
            header_stream
                .map(|(header, pubsub_id)| (header.into(), pubsub_id))
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
            pending_requests: BTreeMap::default(),
            waker: None,
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
        let body_existence_matches_topic = match &block {
            Block::Macro(block) => block.body.is_some(),
            Block::Micro(block) => block.body.is_some() == self.config.include_micro_bodies,
        };
        if !body_existence_matches_topic {
            self.report_validation_result(pubsub_id, MsgAcceptance::Reject);
            return None;
        }

        let blockchain = self.blockchain.read();

        let block_number = block.block_number();
        let head_height = blockchain.block_number();

        // Ignore blocks that we already know.
        if let Ok(info) = blockchain.get_chain_info(&block.hash(), false) {
            self.report_validation_result(
                pubsub_id,
                if info.on_main_chain {
                    MsgAcceptance::Accept
                } else {
                    MsgAcceptance::Ignore
                },
            );
            return None;
        }

        let parent_known = blockchain.contains(block.parent_hash(), true);
        drop(blockchain);

        // Check if a macro block boundary was passed.
        // If so prune the block buffer as well as pending requests.
        let macro_height = Policy::last_macro_block(head_height);
        if macro_height > self.current_macro_height {
            self.current_macro_height = macro_height;
            self.prune_pending_requests();
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
        if !self.insert_block_into_buffer(block, pubsub_id.clone()) {
            log::trace!(
                block_number,
                "Not buffering block - already known or exceeded the per peer limit",
            );
            return;
        }
        log::trace!(block_number, "Buffering block");

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
        self.request_missing_blocks(
            parent_block_number,
            parent_hash,
            None,
            None,
            pubsub_id,
            Direction::Forward,
        );
    }

    /// Attempts to insert a block into the buffer.
    /// This may fail due to the peer who propagated the block as given in the optional `pubsub_id` parameter
    /// is already having too many blocks buffered. Blocks without a pubsub_id will always be buffered.
    ///
    /// At most `MAX_BUFFERED_PER_PEER_PER_HEIGHT` block per peer per block height will be buffered leaving enough
    /// room for a bock and the skip block making it obsolete. For benign peers that is sufficient, as they will
    /// more than that.
    ///
    /// ## Returns
    /// * `true` if the block was added to the buffer
    /// * `false` otherwise
    fn insert_block_into_buffer(&mut self, block: Block, pubsub_id: Option<N::PubsubId>) -> bool {
        let peer_id = match pubsub_id {
            // Blocks without a pubsub_id are requested and will always be buffered.
            None => {
                let map = self.buffer.entry(block.block_number()).or_default();
                if map.contains_key(&block.hash()) {
                    return false;
                }
                assert!(map.insert(block.hash(), (block, None)).is_none());
                return true;
            }
            // For blocks with a pubsub id the propagation source is the peer which can
            // only have MAX_BUFFERED_PER_PEER_PER_HEIGHT items buffered.
            Some(ref pubsub_id) => pubsub_id.propagation_source(),
        };

        // Get the entry for the block number if it exists.
        // Otherwise add the entry and add the block to it and return.
        let map = match self.buffer.entry(block.block_number()) {
            BTreeMapEntry::Occupied(occupied_entry) => occupied_entry.into_mut(),
            BTreeMapEntry::Vacant(vacant_entry) => {
                // Trivially okay to buffer as nothing is buffered yet.
                vacant_entry.insert(HashMap::from([(block.hash(), (block, pubsub_id))]));
                return true;
            }
        };

        let mut blocks_by_peer = 0;
        for (hash, (_block, pubsub)) in map.iter() {
            if *hash == block.hash() {
                // Block is already buffered. Ignore it here.
                return false;
            }

            // Extract the propagation source or move on to the next item
            let propagation_source = match pubsub {
                Some(pubsub_id) => pubsub_id.propagation_source(),
                None => continue,
            };

            // Check if the source is the current peer
            if propagation_source == peer_id {
                // Increase the counter
                blocks_by_peer += 1;
                // Check if the count exceeds the maximum buffer count
                if blocks_by_peer == Self::MAX_BUFFERED_PER_PEER_PER_HEIGHT {
                    log::debug!(
                        ?peer_id, block_number=block.block_number(), dropped_block=?block,
                        "Peer buffer exceeded limit, dropping block",
                    );
                    // If so return, as the peer cannot buffer any additional items.
                    return false;
                }
            }
        }
        // Insert the block. Strictly speaking the .is_none() is redundant with previous checks
        // and could be replaced with return true;
        assert!(map.insert(block.hash(), (block, pubsub_id)).is_none());
        true
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
    ///
    /// `pubsub_id` specifies the gossipsub peer ID of the gossip message that
    /// led to this request, if any. It'll be used as the first peer to request
    /// the missing blocks from.
    fn request_missing_blocks(
        &mut self,
        block_number: u32,
        block_hash: Blake2bHash,
        block_locator: Option<Blake2bHash>,
        epoch_validators: Option<Validators>,
        pubsub_id: Option<N::PubsubId>,
        direction: Direction,
    ) {
        let (head_hash, head_height, macro_height, blocks, epoch_validators) = {
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
            // If we have a set of validators given, we are in a follow-up request and reuse those.
            // If none are given, this request should receive blocks only from the current epoch onwards,
            // so we use the current validator set.
            (
                head_hash,
                head_height,
                macro_height,
                blocks,
                epoch_validators.unwrap_or_else(|| {
                    blockchain
                        .current_validators()
                        .expect("Blockchain does not have a current validator set")
                }),
            )
        };

        if let Ok(blocks) = blocks {
            log::debug!(
                block_number,
                %block_hash,
                %head_hash,
                macro_height,
                "Requesting missing blocks",
            );

            // The block locators are the full batch of micro blocks.
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
                direction,
                epoch_validators,
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
        mut epoch_validators: Validators,
        blocks: Vec<Block>,
    ) -> Option<QueuedBlock<N>> {
        // Verification is already done by the sync queue.
        if blocks.is_empty() {
            return None;
        }

        let last_block = blocks.last()?;
        let block_hash = last_block.hash();
        if block_hash != target_hash {
            // Check if we got a new validator set, otherwise reuse the previous one.
            if last_block.is_election() {
                epoch_validators = last_block.validators()?;
            }
            self.request_missing_blocks(
                target_block_number,
                target_hash,
                Some(block_hash),
                Some(epoch_validators),
                None,
                Direction::Forward,
            );
        }

        let parent_known = {
            let blockchain = self.blockchain.read();
            // Check if the block is still relevant if not discard it
            if blockchain.macro_head().block_number() >= target_block_number {
                return None;
            } else {
                // If the block is relevant, check if the predecessor is known.
                blockchain.contains(blocks[0].parent_hash(), true)
            }
        };

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

    /// Fetches the relevant blocks for any given `BlockchainEvent`
    fn get_new_blocks_from_blockchain_event(&self, event: BlockchainEvent) -> Vec<Block> {
        // Collect block numbers and hashes of newly added blocks first.
        let mut block_infos = vec![];

        match event {
            BlockchainEvent::Extended(block_hash) => {
                if let Ok(block) = self
                    .blockchain
                    .read()
                    .get_block(&block_hash, self.config.include_micro_bodies)
                {
                    block_infos.push(block);
                }
            }
            BlockchainEvent::HistoryAdopted(block_hash)
            | BlockchainEvent::Finalized(block_hash)
            | BlockchainEvent::EpochFinalized(block_hash) => {
                if let Ok(block) = self.blockchain.read().get_block(&block_hash, false) {
                    block_infos.push(block);
                }
            }
            BlockchainEvent::Rebranched(_, new_blocks) => {
                for (_block_hash, block) in new_blocks {
                    block_infos.push(block);
                }
            }
            BlockchainEvent::Stored(block) => {
                block_infos.push(block);
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
                        self.network.validate_message::<BlockHeaderTopic>(
                            id.clone(),
                            MsgAcceptance::Reject,
                        );
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
                    self.network
                        .validate_message::<BlockHeaderTopic>(id.clone(), MsgAcceptance::Ignore);
                }
            }
            false
        });
    }

    /// Cleans up pending requests and removes requests for blocks that precede the current macro block.
    ///
    /// All removed requests will resolve with an Outdated error.
    fn prune_pending_requests(&mut self) {
        self.pending_requests.retain(|&block_number, senders| {
            // Blocks which are after the current macro height are retained as they retain relevance.
            if block_number > self.current_macro_height {
                return true;
            }

            // Resolve all of the pending requests which were removed as outdated.
            // Obviously the requests themselves do not terminate, but they resolve on the caller side. The request
            // yielding a result becomes inconsequential, as the block it would yield can no longer be pushed.
            // This would not be strictly necessary as dropping the sender will resolve the receiving side with a
            // RecvError, but Outdated is more verbose.
            for (_hash, sender) in senders.drain() {
                if let Err(error) = sender.send(Err(ResolveBlockError::Outdated)) {
                    log::warn!(
                        ?error,
                        "Failed to send outdated event for a missing block request"
                    );
                }
            }
            // Remove all entries from the block buffer that precede `current_macro_height`.
            false
        });
    }

    /// For a given collection of blocks check if they resolve a currently pending resolve block request
    fn resolve_pending_requests(&mut self, new_blocks: &Vec<Block>) {
        for new_block in new_blocks {
            if let BTreeMapEntry::Occupied(mut requested_hashes) =
                self.pending_requests.entry(new_block.block_number())
            {
                if let Some(sender) = requested_hashes.get_mut().remove(&new_block.hash()) {
                    if let Err(error) = sender.send(Ok(new_block.clone())) {
                        log::warn!(?error, "Failed to send block for a missing block request");
                    }
                    if requested_hashes.get().is_empty() {
                        requested_hashes.remove();
                    }
                }
            }
        }
    }

    #[inline]
    fn report_validation_result(
        &self,
        pubsub_id: Option<<N as Network>::PubsubId>,
        acceptance: MsgAcceptance,
    ) {
        if let Some(id) = pubsub_id {
            self.network
                .validate_message::<BlockHeaderTopic>(id, acceptance);
        }
    }

    pub(crate) fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        // Deconstruct the request as the parts are needed in different places and for the sender
        // specifically ownership is needed.
        let ResolveBlockRequest::<N> {
            block_number,
            block_hash,
            pubsub_id,
            response_sender,
        } = request;

        // Add the request to pending requests if it does not exists yet.
        // If it already exists, resolve this one with a Duplicate Error.
        match self
            .pending_requests
            .entry(block_number)
            .or_default()
            .entry(block_hash.clone())
        {
            HashMapEntry::Occupied(_entry) => {
                // Already existing request, send the Duplicate Error to resolve this request as
                // the previous one should still do the trick.
                if let Err(error) = response_sender.send(Err(ResolveBlockError::Duplicate)) {
                    log::warn!(
                        ?error,
                        "Failed to send on Oneshot, receiver already dropped"
                    );
                }
                // Do not return as even though the request might not be awaited it should still be executed to
                // try and retrieve the block using the pubsub_id given. It could be the same as in the previous request,
                // but they should be reasonably deduplicated by the network layer. Otherwise it would be a different
                // pubsub_id thus giving more options to actually resolve the block in terms of peers to ask.
            }
            HashMapEntry::Vacant(entry) => {
                entry.insert(response_sender);
            }
        };

        // Check if the block in question is already buffered or pending a push.
        if self
            .buffer
            .get(&block_number)
            .map_or(false, |blocks| blocks.contains_key(&block_hash))
        {
            // Block is already buffered and will be pushed sometime soon. No need to request it.
            return;
        }

        if self.blocks_pending_push.contains(&block_hash) {
            // Block is already pending a push. No need to request it.
            return;
        }

        // The block is relevant and unknown and not requested yet and neither is pending a push. Request the block.
        self.request_missing_blocks(
            block_number,
            block_hash,
            None,
            None,
            Some(pubsub_id),
            Direction::Backward,
        );
    }

    /// Returns a copy of the buffered blocks.
    pub fn buffered_blocks(&self) -> Vec<(u32, Vec<Block>)> {
        self.buffer
            .iter()
            .map(|(block_number, blocks)| {
                (
                    *block_number,
                    blocks
                        .values()
                        .map(|(block, _pubsub_id)| block.clone())
                        .collect(),
                )
            })
            .collect()
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
            let blocks = self.get_new_blocks_from_blockchain_event(event);
            self.resolve_pending_requests(&blocks);
            let block_infos = blocks
                .into_iter()
                .map(|block| (block.block_number(), block.hash()))
                .collect();

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
                    } else {
                        log::warn!(%block, %peer_id, "Rejecting block as it doesn't come from a synced peer");
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
                    epoch_validators,
                    blocks,
                ))) => {
                    if let Some(block) = self.handle_missing_blocks(
                        target_block_number,
                        target_hash,
                        epoch_validators,
                        blocks,
                    ) {
                        return Poll::Ready(Some(block));
                    }
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => break,
            }
        }

        self.waker.store_waker(cx);
        Poll::Pending
    }
}
