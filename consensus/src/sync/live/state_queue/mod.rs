pub mod chunk_request_component;
pub mod live_sync;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::{self, Display},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use beserial::{Deserialize, Serialize};
use futures::{stream::BoxStream, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::Network,
    request::{RequestCommon, RequestMarker},
};
use nimiq_primitives::{
    key_nibbles::KeyNibbles,
    policy::Policy,
    trie::{trie_chunk::TrieChunk, trie_chunk::TrieChunkWithStart},
};
use parking_lot::RwLock;

use self::chunk_request_component::ChunkRequestComponent;
use super::{
    block_queue::{BlockAndId, BlockQueue, QueuedBlock},
    queue::{LiveSyncQueue, QueueConfig},
};

pub struct ChunkAndId<N: Network> {
    pub chunk: TrieChunk,
    pub start_key: KeyNibbles,
    pub peer_id: N::PeerId,
}

impl<N: Network> ChunkAndId<N> {
    pub fn new(chunk: TrieChunk, start_key: KeyNibbles, peer_id: N::PeerId) -> Self {
        Self {
            chunk,
            start_key,
            peer_id,
        }
    }

    pub fn into_pair(self) -> (TrieChunkWithStart, N::PeerId) {
        (
            TrieChunkWithStart {
                chunk: self.chunk,
                start_key: self.start_key,
            },
            self.peer_id,
        )
    }
}

/// The max number of chunk requests per peer.
pub const MAX_REQUEST_RESPONSE_CHUNKS: u32 = 100;

/// The request of a trie chunk.
/// The request specifies a limit on the number of nodes in the chunk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestChunk {
    pub start_key: KeyNibbles,
    pub limit: u32,
}

/// The response for trie chunk requests.
/// In addition to the chunk, we also return the block hash and number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub block_number: u32,
    pub block_hash: Blake2bHash,
    pub chunk: TrieChunk,
}

impl Display for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Chunk {{ block_number: {}, block_hash: {:?}, chunk: {} }}",
            self.block_number, self.block_hash, self.chunk
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[repr(u8)]
pub enum ResponseChunk {
    #[beserial(discriminant = 1)]
    Chunk(Chunk),
    #[beserial(discriminant = 2)]
    IncompleteState,
}

impl Display for ResponseChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResponseChunk::Chunk(chunk) => write!(f, "ResponseChunk::Chunk({})", chunk),
            ResponseChunk::IncompleteState => {
                write!(f, "ResponseChunk::IncompleteState")
            }
        }
    }
}

impl RequestCommon for RequestChunk {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 212;
    type Response = ResponseChunk;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_CHUNKS;
}

pub enum QueuedStateChunks<N: Network> {
    Head(BlockAndId<N>, Vec<ChunkAndId<N>>),
    Buffered(Vec<(BlockAndId<N>, Vec<ChunkAndId<N>>)>),
    Missing(Vec<(Block, Vec<ChunkAndId<N>>)>),
    HeadStateChunk(Vec<ChunkAndId<N>>),
    TooFarFutureBlock(Block, N::PeerId),
    TooDistantPastBlock(Block, N::PeerId),
    TooFarFutureChunk(ChunkAndId<N>),
    TooDistantPastChunk(ChunkAndId<N>),
    PeerIncompleteState(N::PeerId),
}

/// This represents the behavior for the next chunk request. When the accounts trie is:
/// Complete: there are no further requests to make. (Can only set after a macro block).
/// Reset: the next request should start based on the blockchain accounts trie state.
/// Continue: the next request should start on the specified key, which is the end of the previous request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChunkRequestState {
    Complete,
    Paused,
    Reset,
    Continue(KeyNibbles),
}

impl ChunkRequestState {
    pub fn is_complete(&self) -> bool {
        matches!(self, ChunkRequestState::Complete)
    }

    fn with_key_start(start_key: &Option<KeyNibbles>) -> Self {
        if let Some(key) = start_key {
            Self::Continue(key.clone())
        } else {
            Self::Paused
        }
    }

    pub fn should_continue(&self) -> bool {
        matches!(self, ChunkRequestState::Continue(_))
    }
}

pub struct StateQueue<N: Network> {
    /// Configuration for the block queue.
    config: QueueConfig,

    /// Reference to the blockchain.
    blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network.
    network: Arc<N>,

    /// The BlockQueue component.
    block_queue: BlockQueue<N>,

    /// The chunk request component.
    /// We use it to request chunks from up-to-date peers
    chunk_request_component: ChunkRequestComponent<N>,

    /// Buffered chunks - `block_height -> block_hash -> BlockAndId`.
    /// There can be multiple blocks at a height if there are forks.
    /// For each block, we can store multiple chunks.
    buffer: BTreeMap<u32, HashMap<Blake2bHash, Vec<ChunkAndId<N>>>>,

    buffer_size: usize,

    /// The block number of the latest macro block. We prune the block buffer when it changes.
    current_macro_height: u32,

    /// The starting point for the next chunk to be requested. We reset it to `None` when the
    /// chain of chunks is invalidated, which will force the component to request the next
    /// chunk starting from the missing range of the blockchain state.
    /// Invalidation of the chain may happen by an invalid chunk or block, a discarded chunk,
    /// or a rebranch event.
    start_key: ChunkRequestState,

    /// The blockchain event stream.
    blockchain_rx: BoxStream<'static, BlockchainEvent>,
}

impl<N: Network> StateQueue<N> {
    pub fn with_block_queue(
        network: Arc<N>,
        blockchain: Arc<RwLock<Blockchain>>,
        block_queue: BlockQueue<N>,
        config: QueueConfig,
    ) -> Self {
        let chunk_request_component = ChunkRequestComponent::new(
            Arc::clone(&network),
            network.subscribe_events(),
            block_queue.request_component.peer_list(),
        );
        let current_macro_height = Policy::last_macro_block(blockchain.read().block_number());
        let blockchain_rx = blockchain.read().notifier_as_stream();
        Self {
            config,
            blockchain,
            network,
            block_queue,
            chunk_request_component,
            buffer: Default::default(),
            buffer_size: 0,
            current_macro_height,
            start_key: ChunkRequestState::Reset,
            blockchain_rx,
        }
    }

    pub fn remove_invalid_blocks(&mut self, invalid_blocks: &mut HashSet<Blake2bHash>) {
        // First we remove invalid blocks from the blockqueue.
        self.block_queue.remove_invalid_blocks(invalid_blocks);

        if invalid_blocks.is_empty() {
            return;
        }

        // Iterate over block buffer, remove element if no blocks remain at that height.
        self.buffer.retain(|_block_number, blocks| {
            // Iterate over all blocks at the current height, remove block if blockqueue marked it as invalid.
            blocks.retain(|hash, _chunks| {
                if invalid_blocks.contains(hash) {
                    self.buffer_size -= 1;
                    return false;
                }
                true
            });
            !blocks.is_empty()
        });
    }

    /// Resets the starting key for requesting the next chunks.
    /// When reset this component uses the blockchain state missing range start key.
    pub fn reset_chunk_request_chain(&mut self) {
        self.start_key = ChunkRequestState::Reset;
    }

    pub fn on_block_processed(&mut self, hash: &Blake2bHash) {
        self.block_queue.on_block_processed(hash)
    }

    pub fn buffered_chunks_len(&self) -> usize {
        self.buffer_size
    }

    pub fn buffered_blocks_len(&self) -> usize {
        self.block_queue.buffered_blocks_len()
    }

    pub fn chunk_request_state(&self) -> &ChunkRequestState {
        &self.start_key
    }

    /// Requests a chunk from the peers starting at the starting key from this queue.
    /// If no starting key is supplied we start at the missing range from the blockchain.
    /// If no starting key is supplied and the trie is already complete no request is being made.
    fn request_chunk(&mut self) -> bool {
        let start_key = match self.start_key {
            ChunkRequestState::Complete | ChunkRequestState::Paused => None,
            ChunkRequestState::Reset => self
                .blockchain
                .read()
                .get_missing_accounts_range(None)
                .map(|v| v.start),
            ChunkRequestState::Continue(ref key) => Some(key.clone()),
        };

        if let Some(start_key) = start_key {
            let req = RequestChunk {
                start_key: start_key.clone(),
                limit: Policy::state_chunks_max_size(),
            };
            self.chunk_request_component.request_chunk(req);
            self.start_key = ChunkRequestState::Continue(start_key);
            return true;
        }
        false
    }

    fn insert_chunk_into_buffer(
        &mut self,
        response: Chunk,
        start_key: KeyNibbles,
        peer_id: N::PeerId,
    ) {
        let chunks = self
            .buffer
            .entry(response.block_number)
            .or_default()
            .entry(response.block_hash.clone())
            .or_default();

        // We try to avoid duplicate chunks by checking start and end key
        // as well as items length for efficiency reasons.
        if chunks.iter().any(|chunk| {
            chunk.start_key == start_key
                && chunk.chunk.items.len() == response.chunk.items.len()
                && chunk.chunk.end_key == response.chunk.end_key
        }) {
            log::debug!(
                "Discarding duplicate chunk {} for block (#{}, hash {})",
                response.chunk,
                response.block_number,
                response.block_hash
            );
            return;
        }

        chunks.push(ChunkAndId::new(response.chunk, start_key, peer_id));
        self.buffer_size += 1;
    }

    /// Order them inside our buffer if enough space and inside window.
    fn on_chunk_received(
        &mut self,
        chunk: ResponseChunk,
        start_key: KeyNibbles,
        peer_id: N::PeerId,
    ) -> Option<QueuedStateChunks<N>> {
        // Filter for peers with incomplete state first.
        let chunk = match chunk {
            ResponseChunk::IncompleteState => {
                return Some(QueuedStateChunks::PeerIncompleteState(peer_id))
            }
            ResponseChunk::Chunk(chunk) => chunk,
        };

        let blockchain = self.blockchain.read();
        let current_block_height = blockchain.block_number();
        let current_block_hash = blockchain.head_hash();
        let contains_block = blockchain.contains(&chunk.block_hash, true);
        drop(blockchain);

        // Check if a macro block boundary was passed. If so prune the block buffer.
        let macro_height = Policy::last_macro_block(current_block_height);
        if macro_height > self.current_macro_height {
            self.current_macro_height = macro_height;
            self.prune_buffer();
        }

        if chunk.block_number < current_block_height.saturating_sub(self.config.tolerate_past_max) {
            log::warn!(
                "Discarding chunk {} earlier than toleration window (max {})",
                chunk,
                current_block_height.saturating_sub(self.config.tolerate_past_max),
            );

            if self.network.has_peer(peer_id) {
                self.chunk_request_component.remove_peer(&peer_id);
                return Some(QueuedStateChunks::TooDistantPastChunk(ChunkAndId::new(
                    chunk.chunk,
                    start_key,
                    peer_id,
                )));
            }
        } else if chunk.block_number > current_block_height + self.config.window_ahead_max {
            log::warn!(
                "Discarding chunk {} outside of buffer window (max {})",
                chunk,
                current_block_height + self.config.window_ahead_max,
            );

            if self.network.has_peer(peer_id) {
                self.chunk_request_component.remove_peer(&peer_id);
                return Some(QueuedStateChunks::TooFarFutureChunk(ChunkAndId::new(
                    chunk.chunk,
                    start_key,
                    peer_id,
                )));
            }
        } else if self.buffer_size >= self.config.buffer_max {
            log::warn!(
                "Discarding chunk {}, buffer full (max {})",
                chunk,
                self.buffer_size,
            );
        } else if chunk.block_hash == current_block_hash {
            // Immediately return chunks for the current head blockchain.
            self.set_start_key(&chunk.chunk.end_key);
            return Some(QueuedStateChunks::HeadStateChunk(vec![ChunkAndId::new(
                chunk.chunk,
                start_key,
                peer_id,
            )]));
        } else if chunk.block_number <= macro_height {
            // Chunk is from a previous batch/epoch, discard it.
            log::warn!(
                "Discarding chunk {}, we're already at macro block #{}",
                chunk,
                macro_height
            );
        } else if contains_block {
            // Block is already in blockchain, cannot apply chunk so we discard it.
            log::debug!("Discarding chunk {}, block already applied", chunk);
        } else {
            // Chunk is inside the buffer window, put it in the buffer.
            self.set_start_key(&chunk.chunk.end_key);
            self.insert_chunk_into_buffer(chunk, start_key, peer_id);
        }
        None
    }

    /// Handles blocks received from the blockqueue.
    /// Upon a new block, we return it with any chunks we already have.
    /// If we receive multiple blocks at once, we always return the first and buffer the rest.
    fn on_blocks_received(&mut self, queued_block: QueuedBlock<N>) -> QueuedStateChunks<N> {
        match queued_block {
            // Received a single block and retrieve the corresponding chunks.
            QueuedBlock::Head((block, pubsub_id)) => {
                let chunks = self.get_block_chunks(&block);
                QueuedStateChunks::Head((block, pubsub_id), chunks)
            }
            QueuedBlock::Buffered(blocks) => {
                // Received multiple blocks, get the chunks avl for all blocks.
                let blocks_and_chunks = blocks
                    .into_iter()
                    .map(|(block, pubsub_id)| {
                        let chunks = self.get_block_chunks(&block);
                        ((block, pubsub_id), chunks)
                    })
                    .collect();
                QueuedStateChunks::Buffered(blocks_and_chunks)
            }
            QueuedBlock::Missing(missing_blocks) => {
                // Received multiple blocks, get the chunks avl for all blocks.
                let blocks_and_chunks = missing_blocks
                    .into_iter()
                    .map(|block| {
                        let chunks = self.get_block_chunks(&block);
                        (block, chunks)
                    })
                    .collect();
                QueuedStateChunks::Missing(blocks_and_chunks)
            }
            // Received too far away blocks (past). We forward the block queue event without chunks.
            QueuedBlock::TooFarBehind(block, peer_id) => {
                QueuedStateChunks::TooDistantPastBlock(block, peer_id)
            }
            // Received too far away blocks (future). We forward the block queue event without chunks.
            QueuedBlock::TooFarAhead(block, peer_id) => {
                QueuedStateChunks::TooFarFutureBlock(block, peer_id)
            }
        }
    }

    fn get_block_chunks(&mut self, block: &Block) -> Vec<ChunkAndId<N>> {
        let mut chunks = vec![];
        let mut is_empty = false;
        if let Some(blocks) = self.buffer.get_mut(&block.block_number()) {
            if let Some(buffered_chunks) = blocks.remove(&block.hash()) {
                chunks = buffered_chunks;
                self.buffer_size -= chunks.len();
            }
            is_empty = blocks.is_empty();
        }

        if is_empty {
            self.buffer.remove(&block.block_number());
        }

        chunks
    }

    fn prune_buffer(&mut self) {
        self.buffer.retain(|&block_number, blocks| {
            // Remove all entries from the block buffer that precede `current_macro_height`.
            if block_number <= self.current_macro_height {
                self.buffer_size -= blocks.values().map(|chunks| chunks.len()).sum::<usize>();
                return false;
            }
            true
        });
    }

    /// Sets the start key except if the chain of chunks has been invalidated.
    fn set_start_key(&mut self, start_key: &Option<KeyNibbles>) {
        if self.start_key.should_continue() {
            self.start_key = ChunkRequestState::with_key_start(start_key);
        }
    }
}

impl<N: Network> Stream for StateQueue<N> {
    type Item = QueuedStateChunks<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll the blockchain stream and set key start to complete if the accounts trie is
        // complete and we have passed a macro block.
        // If we have not passed the macro block yet, reset on a rebranch (since we potentially revert from an incomplete state).
        while let Poll::Ready(Some(event)) = self.blockchain_rx.poll_next_unpin(cx) {
            if !self.start_key.is_complete() {
                match event {
                    BlockchainEvent::Finalized(_) | BlockchainEvent::EpochFinalized(_) => {
                        if self.blockchain.read().state.accounts.is_complete(None) {
                            info!("Finished state sync, trie complete.");
                            self.start_key = ChunkRequestState::Complete;
                            self.buffer.clear();
                            self.buffer_size = 0;
                        }
                    }
                    BlockchainEvent::Rebranched(_, _) => {
                        if !self.blockchain.read().state.accounts.is_complete(None) {
                            info!("Reset due to rebranch.");
                            self.start_key = ChunkRequestState::Reset;
                        }
                    }
                    _ => {}
                }
            }
        }

        // 1. Receive chunks from ChunkRequestComponent.
        loop {
            match self.chunk_request_component.poll_next_unpin(cx) {
                Poll::Ready(Some((chunk, start_key, peer_id))) => {
                    if let Some(state_chunks) = self.on_chunk_received(chunk, start_key, peer_id) {
                        return Poll::Ready(Some(state_chunks));
                    }
                }
                // If the chunk stream is exhausted, we quit as well.
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => break,
            }
        }

        // 2. Request chunks via ChunkRequestComponent.
        if !self.chunk_request_component.has_pending_requests() && self.num_peers() > 0 {
            self.request_chunk();
        }

        // 3. Receive blocks from BlockQueue.
        let poll_res = self.block_queue.poll_next_unpin(cx);
        match poll_res {
            Poll::Ready(Some(queued_block)) => {
                return Poll::Ready(Some(self.on_blocks_received(queued_block)));
            }
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        Poll::Pending
    }
}
