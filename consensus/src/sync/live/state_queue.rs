use std::{
    collections::{BTreeMap, HashMap, HashSet},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use beserial::{Deserialize, Serialize};
use futures::{Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::Network,
    request::{RequestCommon, RequestMarker},
};
use nimiq_primitives::policy::Policy;
use nimiq_trie::{key_nibbles::KeyNibbles, trie::TrieChunk};
use parking_lot::RwLock;

use super::{
    block_queue::{BlockQueue, QueuedBlock},
    block_request_component::RequestComponent,
    chunk_request_component::ChunkRequestComponent,
    BlockQueueConfig,
};

pub(crate) type ChunkAndId<N> = (TrieChunk, <N as Network>::PeerId);

/// The max number of chunk requests per peer.
pub const MAX_REQUEST_RESPONSE_CHUNKS: u32 = 100;

/// The request of a trie chunk.
/// The request specifies a limit on the number of nodes in the chunk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestChunk {
    start_key: KeyNibbles,
    limit: u32,
}

/// The response for trie chunk requests.
/// In addition to the chunk, we also return the block hash and number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseChunk {
    block_number: u32,
    block_hash: Blake2bHash,
    chunk: TrieChunk,
}

impl RequestCommon for RequestChunk {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 212;
    type Response = ResponseChunk;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_CHUNKS;
}

pub enum QueuedStateChunks<N: Network> {
    StateChunk(QueuedBlock<N>, Vec<ChunkAndId<N>>),
    HeadStateChunk(Vec<ChunkAndId<N>>),
    TooFarFuture(ChunkAndId<N>),
    TooDistantPast(ChunkAndId<N>),
}

pub struct StateQueue<N: Network, TReq: RequestComponent<N>> {
    /// Configuration for the block queue
    config: BlockQueueConfig,

    /// Reference to the blockchain
    blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network
    network: Arc<N>,

    /// The BlockQueue component
    block_queue: BlockQueue<N, TReq>,

    /// The chunk request component.
    /// We use it to request chunks from up-to-date peers
    chunk_request_component: ChunkRequestComponent<N>,

    /// Buffered chunks - `block_height -> block_hash -> BlockAndId`.
    /// There can be multiple blocks at a height if there are forks.
    /// For each block, we can store multiple chunks.
    buffer: BTreeMap<u32, HashMap<Blake2bHash, Vec<ChunkAndId<N>>>>,

    buffer_size: usize,

    /// Blocks to be processed and returned by the stream
    pending_blocks: Option<QueuedBlock<N>>,

    /// Hashes of blocks that are pending to be pushed to the chain.
    // pending_chunks: BTreeSet<Blake2bHash>,
    // waker: Option<Waker>,

    /// The block number of the latest macro block. We prune the block buffer when it changes.
    current_macro_height: u32,
}

impl<N: Network, TReq: RequestComponent<N>> StateQueue<N, TReq> {
    pub fn remove_invalid_blocks(&mut self, invalid_blocks: &mut HashSet<Blake2bHash>) {
        // First we remove invalid blocks from the blockqueue
        self.block_queue.remove_invalid_blocks(invalid_blocks);

        if invalid_blocks.is_empty() {
            return;
        }

        // Iterate over block buffer, remove element if no blocks remain at that height.
        self.buffer.retain(|_block_number, blocks| {
            // Iterate over all blocks at the current height, remove block if blockqueue marked it as invalid
            blocks.retain(|hash, _chunks| !invalid_blocks.contains(hash));
            !blocks.is_empty()
        });
    }

    fn request_chunks(&self) {}

    fn insert_chunk_into_buffer(&mut self, response: ResponseChunk, peer_id: N::PeerId) {
        self.buffer
            .entry(response.block_number)
            .or_default()
            .entry(response.block_hash)
            .or_default()
            .push((response.chunk, peer_id));
        self.buffer_size += 1;
    }

    /// Order them inside our buffer if enough space and inside window
    fn on_chunk_received(
        &mut self,
        chunk: ResponseChunk,
        peer_id: N::PeerId,
    ) -> Option<QueuedStateChunks<N>> {
        let blockchain = self.blockchain.read();
        let current_block_height = blockchain.block_number();
        let current_block_hash = blockchain.head_hash();
        drop(blockchain);

        // Check if a macro block boundary was passed. If so prune the block buffer.
        let macro_height = Policy::last_macro_block(current_block_height);
        if macro_height > self.current_macro_height {
            self.current_macro_height = macro_height;
            self.prune_buffer();
        }

        if chunk.block_number < current_block_height.saturating_sub(self.config.tolerate_past_max) {
            log::warn!(
                "Discarding chunk {:?} earlier than toleration window (max {})",
                chunk,
                current_block_height.saturating_sub(self.config.tolerate_past_max),
            );

            if self.network.has_peer(peer_id) {
                self.chunk_request_component.remove_peer(&peer_id);
                return Some(QueuedStateChunks::TooDistantPast((chunk.chunk, peer_id)));
            }
        } else if chunk.block_number > current_block_height + self.config.window_ahead_max {
            log::warn!(
                "Discarding chunk {:?} outside of buffer window (max {})",
                chunk,
                current_block_height + self.config.window_ahead_max,
            );

            if self.network.has_peer(peer_id) {
                self.chunk_request_component.remove_peer(&peer_id);
                return Some(QueuedStateChunks::TooFarFuture((chunk.chunk, peer_id)));
            }
        } else if self.buffer_size >= self.config.buffer_max {
            log::warn!(
                "Discarding chunk {:?}, buffer full (max {})",
                chunk,
                self.buffer_size,
            );
        } else if chunk.block_number <= macro_height {
            // Chunk is from a previous batch/epoch, discard it.
            log::warn!(
                "Discarding chunk {:?}, we're already at macro block #{}",
                chunk,
                macro_height
            );
        } else if chunk.block_hash == current_block_hash {
            // PITODO check that chunk is next in line
            // Immediately return chunks for the current head blockchain.
            return Some(QueuedStateChunks::HeadStateChunk(vec![(
                chunk.chunk,
                peer_id,
            )]));
        } else {
            // Chunk is inside the buffer window, put it in the buffer.
            self.insert_chunk_into_buffer(chunk, peer_id);
        }
        None
    }

    /// Handles blocks received from the blockqueue.
    /// Upon a new block, we return it with any chunks we already have.
    /// If we receive multiple blocks at once, we always return the first and buffer the rest.
    fn on_blocks_received(&mut self, queued_block: QueuedBlock<N>) -> Option<QueuedStateChunks<N>> {
        let chunks = match queued_block {
            // Received a single block and retrieve the corresponding chunks
            QueuedBlock::Head((ref block, ref _peer_id)) => self.get_block_chunks(&block),
            QueuedBlock::Buffered(blocks) => {
                todo!() // PITODO: Handle this case
            }
            QueuedBlock::Missing(mut missing_blocks) => {
                // Received multiple blocks, get the chunks avl for the first and queue the remaining blocks
                if let Some(block) = missing_blocks.pop() {
                    let chunks = self.get_block_chunks(&block);
                    if !missing_blocks.is_empty() {
                        assert!(
                            self.pending_blocks.is_none(),
                            "We always handle pending blocks before polling new ones."
                        );
                        self.pending_blocks = Some(QueuedBlock::Missing(missing_blocks));
                    }

                    return Some(QueuedStateChunks::StateChunk(
                        QueuedBlock::Missing(vec![block]),
                        chunks,
                    ));
                } else {
                    log::error!("Received empty missing blocks from block queue");
                    return None;
                };
            }
            // Received too far away blocks (future or past). We forward the block queue event without chunks.
            _ => vec![],
        };

        Some(QueuedStateChunks::StateChunk(queued_block, chunks))
    }

    fn get_block_chunks(&mut self, block: &Block) -> Vec<(TrieChunk, N::PeerId)> {
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
}

impl<N: Network, TReq: RequestComponent<N>> Stream for StateQueue<N, TReq> {
    type Item = QueuedStateChunks<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // 1. Receive chunks from ChunkRequestComponent
        loop {
            match self.chunk_request_component.poll_next_unpin(cx) {
                Poll::Ready(Some((chunk, peer_id))) => {
                    if let Some(state_chunks) = self.on_chunk_received(chunk, peer_id) {
                        return Poll::Ready(Some(state_chunks));
                    }
                }
                // If the chunk stream is exhausted, we quit as well.
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => break,
            }
        }

        // 2. Process and return buffered blocks (can happen if block queue returns a vector of blocks)
        if let Some(queued_block) = self.pending_blocks.take() {
            if let Some(state_chunks) = self.on_blocks_received(queued_block) {
                return Poll::Ready(Some(state_chunks));
            }
        }

        // 3. Request chunks via ChunkRequestComponent
        self.request_chunks();

        // 4. Receive blocks from BlockQueue
        loop {
            let poll_res = self.block_queue.poll_next_unpin(cx);
            match poll_res {
                Poll::Ready(Some(queued_block)) => {
                    if let Some(state_chunks) = self.on_blocks_received(queued_block) {
                        return Poll::Ready(Some(state_chunks));
                    }
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => break,
            }
        }

        Poll::Pending
    }
}
