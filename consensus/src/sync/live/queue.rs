use std::{
    collections::{HashSet, VecDeque},
    fmt,
    sync::Arc,
};

use futures::{future::BoxFuture, FutureExt, Stream};
use nimiq_block::Block;
#[cfg(feature = "full")]
use nimiq_blockchain::{Blockchain, PostValidationHook};
#[cfg(feature = "full")]
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_interface::{ChunksPushError, ChunksPushResult, PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::{MsgAcceptance, Network};
use nimiq_primitives::{
    key_nibbles::KeyNibbles,
    policy::Policy,
    trie::{
        trie_chunk::{TrieChunk, TrieChunkWithStart},
        trie_diff::TrieDiff,
    },
};
use parking_lot::Mutex;

use crate::{
    consensus::ResolveBlockRequest,
    sync::{
        live::block_queue::{BlockAndSource, BlockSource},
        syncer::LiveSyncEvent,
    },
};

async fn spawn_blocking<R: Send + 'static, F: FnOnce() -> R + Send + 'static>(f: F) -> R {
    #[cfg(not(target_family = "wasm"))]
    {
        tokio::task::spawn_blocking(f).await.unwrap()
    }

    #[cfg(target_family = "wasm")]
    {
        f()
    }
}

pub struct ChunkAndSource<N: Network> {
    pub chunk: TrieChunk,
    pub start_key: KeyNibbles,
    pub peer_id: N::PeerId,
}

impl<N: Network> fmt::Debug for ChunkAndSource<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ChunkAndSource")
            .field("chunk", &self.chunk)
            .field("start_key", &self.start_key)
            .field("peer_id", &self.peer_id)
            .finish()
    }
}

impl<N: Network> ChunkAndSource<N> {
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

pub trait LiveSyncQueue<N: Network>: Stream<Item = Self::QueueResult> + Send + Unpin {
    type QueueResult: Send;
    type PushResult;

    fn push_queue_result(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        result: Self::QueueResult,
    ) -> VecDeque<BoxFuture<'static, Self::PushResult>>;

    fn process_push_result(&mut self, item: Self::PushResult) -> Option<LiveSyncEvent<N::PeerId>>;

    fn peers(&self) -> Vec<N::PeerId>;

    fn num_peers(&self) -> usize;

    fn add_peer(&self, peer_id: N::PeerId);

    /// Adds a block stream by replacing the current block stream with a `select` of both streams.
    fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = BlockAndSource<N>> + Send + 'static;

    fn include_body(&self) -> bool;

    fn state_complete(&self) -> bool {
        true
    }

    /// Initiates an attempt to resolve a ResolveBlockRequest.
    fn resolve_block(&mut self, request: ResolveBlockRequest<N>);

    /// The maximum number of blocks a peer can be ahead before it is considered out-of-sync.
    fn acceptance_window_size(&self) -> u32;
}

#[derive(Clone, Debug)]
pub struct QueueConfig {
    /// Buffer size limit
    pub buffer_max: usize,

    /// How many blocks ahead we will buffer.
    pub window_ahead_max: u32,

    /// How many blocks back into the past we tolerate without returning a peer as Outdated.
    pub tolerate_past_max: u32,

    /// Flag to indicate if blocks should carry a body.
    pub include_body: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            buffer_max: 10 * Policy::blocks_per_batch() as usize,
            window_ahead_max: 2 * Policy::blocks_per_batch(),
            tolerate_past_max: Policy::blocks_per_batch(),
            include_body: true,
        }
    }
}

struct BlockchainPushResult<N: Network> {
    block_push_result: Option<Result<PushResult, PushError>>,
    push_chunks_result: Result<ChunksPushResult, ChunksPushError>,
    #[allow(dead_code)] // TODO ban peer related
    chunk_error_peer: Option<<N as Network>::PeerId>,
    block_hash: Blake2bHash,
}

impl<N: Network> BlockchainPushResult<N> {
    fn with_light_block_result(
        push_result: Result<PushResult, PushError>,
        block_hash: Blake2bHash,
    ) -> Self {
        Self {
            block_push_result: Some(push_result),
            push_chunks_result: Ok(ChunksPushResult::EmptyChunks),
            block_hash,
            chunk_error_peer: None,
        }
    }

    #[cfg(feature = "full")]
    fn with_block_result(
        push_result: Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError>,
        block_hash: Blake2bHash,
        peer_ids: &[N::PeerId],
    ) -> Self {
        let (block_push_result, push_chunks_result, chunk_error_peer) = match push_result {
            Ok((block_push_result, Err(chunk_error))) => {
                let peer_id = peer_ids[chunk_error.chunk_index()];
                (Ok(block_push_result), Err(chunk_error), Some(peer_id))
            }
            Ok((block_push_result, push_chunks_result)) => {
                (Ok(block_push_result), push_chunks_result, None)
            }
            Err(push_error) => (Err(push_error), Ok(ChunksPushResult::EmptyChunks), None),
        };

        Self {
            block_push_result: Some(block_push_result),
            push_chunks_result,
            block_hash,
            chunk_error_peer,
        }
    }

    #[cfg(feature = "full")]
    fn with_chunks_result(
        push_chunks_result: Result<ChunksPushResult, ChunksPushError>,
        block_hash: Blake2bHash,
        peer_ids: &[N::PeerId],
    ) -> Self {
        let peer_id = if let Err(ref chunk_error) = push_chunks_result {
            let peer_id = peer_ids[chunk_error.chunk_index()];
            Some(peer_id)
        } else {
            None
        };

        Self {
            block_push_result: None,
            push_chunks_result,
            block_hash,
            chunk_error_peer: peer_id,
        }
    }
}

/// Pushes a single block and respective chunks into the blockchain and validates the message.
#[cfg(feature = "full")]
pub async fn push_block_and_chunks<N: Network>(
    network: Arc<N>,
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    block: Block,
    block_source: BlockSource<N>,
    diff: Option<TrieDiff>,
    chunks: Vec<ChunkAndSource<N>>,
) -> (
    Result<PushResult, PushError>,
    Result<ChunksPushResult, ChunksPushError>,
    Blake2bHash,
) {
    let push_results = spawn_blocking(move || {
        blockchain_push(
            blockchain,
            bls_cache,
            Some(block),
            diff,
            chunks,
            Some(MessageValidator {
                network,
                block_source,
            }),
        )
    })
    .await;

    // TODO Ban peer depending on type of chunk error?

    (
        push_results.block_push_result.unwrap(),
        push_results.push_chunks_result,
        push_results.block_hash,
    )
}

/// Pushes a single block into the blockchain and validates the message.
pub async fn push_block_only<N: Network>(
    network: Arc<N>,
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    block: Block,
    block_source: BlockSource<N>,
) -> (Result<PushResult, PushError>, Blake2bHash) {
    let push_results = spawn_blocking(move || {
        blockchain_push::<N>(
            blockchain,
            bls_cache,
            Some(block),
            None,
            vec![],
            Some(MessageValidator {
                network,
                block_source,
            }),
        )
    })
    .await;

    (
        push_results.block_push_result.unwrap(),
        push_results.block_hash,
    )
}

/// Pushes a sequence of blocks to the blockchain.
/// This case is different from pushing single blocks in a for loop,
/// because an invalid block automatically invalidates the remainder of the sequence.
pub async fn push_multiple_blocks_impl<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    blocks: Vec<(BlockAndSource<N>, Option<TrieDiff>, Vec<ChunkAndSource<N>>)>,
) -> (
    Result<PushResult, PushError>,
    Result<ChunksPushResult, ChunksPushError>,
    Vec<Blake2bHash>,
    HashSet<Blake2bHash>,
) {
    let mut block_iter = blocks.into_iter();
    // Hashes of adopted blocks
    let mut adopted_blocks = Vec::new();
    // Hashes of invalid blocks
    let mut invalid_blocks = HashSet::new();
    // Initialize push_result to some random value.
    // It is always overwritten in the first loop iteration.
    let mut push_result = Err(PushError::Orphan);
    let mut push_chunk_result = Ok(ChunksPushResult::EmptyChunks);
    // Try to push blocks, until we encounter an invalid block.
    for ((block, _), diff, mut chunks) in block_iter.by_ref() {
        log::debug!("Pushing block {} from missing blocks response", block);

        let blockchain2 = blockchain.clone();
        let bls_cache2 = Arc::clone(&bls_cache);

        // If a previous chunk failed, we should discard all subsequent chunks.
        if push_chunk_result.is_err() {
            chunks.clear();
        }
        let push_results = spawn_blocking(move || {
            blockchain_push::<N>(blockchain2, bls_cache2, Some(block), diff, chunks, None)
        })
        .await;

        push_result = push_results.block_push_result.unwrap();
        let block_hash = push_results.block_hash;

        // The chunk result should give precedence to an error. Otherwise, if least one chunk was pushed,
        // the result should reflect that.
        // Errors cannot be overwritten because we will discard the subsequent chunks.
        match push_results.push_chunks_result {
            Ok(ChunksPushResult::Chunks(committed, ignored)) => match push_chunk_result {
                Ok(ChunksPushResult::Chunks(ref mut old_committed, ref mut old_ignored)) => {
                    *old_committed += committed;
                    *old_ignored += ignored;
                }
                Ok(_) => push_chunk_result = push_results.push_chunks_result,
                Err(_) => unreachable!(),
            },
            Err(_) => push_chunk_result = push_results.push_chunks_result,
            Ok(_) => {}
        }

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

    // TODO Ban peer depending on type of chunk error?

    // If there are remaining blocks in the iterator, those are invalid.
    for ((block, _), ..) in block_iter {
        invalid_blocks.insert(block.hash());
    }
    (
        push_result,
        push_chunk_result,
        adopted_blocks,
        invalid_blocks,
    )
}

pub async fn push_multiple_blocks_with_chunks<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    blocks: Vec<(BlockAndSource<N>, Option<TrieDiff>, Vec<ChunkAndSource<N>>)>,
) -> (
    Result<PushResult, PushError>,
    Result<ChunksPushResult, ChunksPushError>,
    Vec<Blake2bHash>,
    HashSet<Blake2bHash>,
) {
    push_multiple_blocks_impl(blockchain, bls_cache, blocks).await
}

/// Pushes a sequence of blocks to the blockchain.
/// This case is different from pushing single blocks in a for loop,
/// because an invalid block automatically invalidates the remainder of the sequence.
pub async fn push_multiple_blocks<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    blocks: Vec<BlockAndSource<N>>,
) -> (
    Result<PushResult, PushError>,
    Vec<Blake2bHash>,
    HashSet<Blake2bHash>,
) {
    let blocks = blocks
        .into_iter()
        .map(|block| (block, None, vec![]))
        .collect();
    push_multiple_blocks_impl::<N>(blockchain, bls_cache, blocks)
        .map(|(push_result, _, adopted_blocks, invalid_blocks)| {
            (push_result, adopted_blocks, invalid_blocks)
        })
        .await
}

/// Pushes the chunks to the current blockchain state.
#[cfg(feature = "full")]
pub async fn push_chunks_only<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    chunks: Vec<ChunkAndSource<N>>,
) -> (Result<ChunksPushResult, ChunksPushError>, Blake2bHash) {
    let push_results =
        spawn_blocking(move || blockchain_push(blockchain, bls_cache, None, None, chunks, None))
            .await;

    // TODO Ban peer depending on type of chunk error?

    (push_results.push_chunks_result, push_results.block_hash)
}

pub struct MessageValidator<N: Network> {
    network: Arc<N>,
    block_source: BlockSource<N>,
}

#[cfg(feature = "full")]
impl<N: Network> PostValidationHook for MessageValidator<N> {
    fn post_validation(&self, push_result: Result<&PushResult, &PushError>) {
        let acceptance = match push_result {
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

        self.block_source.validate_block(&self.network, acceptance);
    }
}

/// Pushes the a single block and the respective chunks into the blockchain. If a light
/// blockchain was supplied, no chunks are committed.
/// The return value consists of the result of pushing the block, the error of pushing
/// the chunks and the block hash. If no block or chunks are supplied no change will
/// happen to the blockchain and the return will look like `(None, None, current_head_hash)`.
/// Note: this function doesn't prevent a new block from being committed before applying the chunks.
fn blockchain_push<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    block: Option<Block>,
    diff: Option<TrieDiff>,
    chunks: Vec<ChunkAndSource<N>>,
    msg_validator: Option<MessageValidator<N>>,
) -> BlockchainPushResult<N> {
    #[cfg(feature = "full")]
    let (chunks, peer_ids): (Vec<_>, Vec<N::PeerId>) =
        chunks.into_iter().map(ChunkAndSource::into_pair).unzip();

    // Push the block to the blockchain.
    let blockchain_push_result;
    if let Some(block) = block {
        let block_hash = block.hash();
        // Update validator keys from BLS public key cache.
        block.update_validator_keys(&mut bls_cache.lock());
        match blockchain {
            #[cfg(feature = "full")]
            BlockchainProxy::Full(ref blockchain) => {
                // We push the block and if it fails we return immediately, without committing chunks.
                let push_result = match diff {
                    Some(diff) => Blockchain::push_with_chunks(
                        blockchain.upgradable_read(),
                        block,
                        diff,
                        chunks,
                        &msg_validator,
                    ),
                    None => {
                        assert!(chunks.is_empty());
                        Blockchain::push_with_hook(
                            blockchain.upgradable_read(),
                            block,
                            &msg_validator,
                        )
                        .map(|r| (r, Ok(ChunksPushResult::EmptyChunks)))
                    }
                };

                blockchain_push_result =
                    BlockchainPushResult::with_block_result(push_result, block_hash, &peer_ids);
            }
            BlockchainProxy::Light(ref blockchain) => {
                let push_result = LightBlockchain::push(blockchain.upgradable_read(), block);

                blockchain_push_result =
                    BlockchainPushResult::with_light_block_result(push_result, block_hash);
            }
        }
    } else {
        match blockchain {
            #[cfg(feature = "full")]
            BlockchainProxy::Full(ref blockchain) => {
                let bc = blockchain.upgradable_read();
                let block_hash = bc.head_hash();
                // We push the chunks.
                let chunks_push_result = bc.commit_chunks(chunks, &block_hash);
                blockchain_push_result = BlockchainPushResult::with_chunks_result(
                    chunks_push_result,
                    block_hash,
                    &peer_ids,
                );
            }
            BlockchainProxy::Light(ref _blockchain) => {
                // We do not push chunks into a light blockchain.
                unreachable!()
            }
        }
    }

    blockchain_push_result
}
