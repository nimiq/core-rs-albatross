use futures::{future::BoxFuture, FutureExt, Stream};
use nimiq_primitives::policy::Policy;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};
use tokio::task::spawn_blocking;

use parking_lot::Mutex;

use nimiq_block::{Block, BlockHeaderTopic, BlockTopic};
use nimiq_blockchain::{Blockchain, ChunksPushError, ChunksPushResult};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::{MsgAcceptance, Network};

use crate::sync::syncer::LiveSyncEvent;

use super::state_queue::ChunkAndId;

pub trait LiveSyncQueue<N: Network>: Stream<Item = Self::QueueResult> + Send + Unpin {
    type QueueResult;
    type PushResult;

    fn push_queue_result(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        result: Self::QueueResult,
        include_body: bool,
    ) -> VecDeque<BoxFuture<'static, Self::PushResult>>;

    fn process_push_result(&mut self, item: Self::PushResult) -> Option<LiveSyncEvent<N::PeerId>>;

    fn num_peers(&self) -> usize;

    fn include_micro_bodies(&self) -> bool;

    fn peers(&self) -> Vec<N::PeerId>;

    fn add_peer(&self, peer_id: N::PeerId);

    /// Adds an additional block stream by replacing the current block stream with a `select` of both streams.
    fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = (Block, N::PeerId, Option<N::PubsubId>)> + Send + 'static;

    fn state_complete(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug)]
pub struct QueueConfig {
    /// Buffer size limit
    pub buffer_max: usize,

    /// How many blocks ahead we will buffer.
    pub window_ahead_max: u32,

    /// How many blocks back into the past we tolerate without returning a peer as Outdated.
    pub tolerate_past_max: u32,

    /// Flag to indicate if blocks should carry a body
    pub include_micro_bodies: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            buffer_max: 4 * Policy::blocks_per_batch() as usize,
            window_ahead_max: 2 * Policy::blocks_per_batch(),
            tolerate_past_max: Policy::blocks_per_batch(),
            include_micro_bodies: true,
        }
    }
}

struct BlockchainPushResult<N: Network> {
    block_push_result: Option<Result<PushResult, PushError>>,
    push_chunks_result: Result<ChunksPushResult, ChunksPushError>,
    chunk_error_peer: Option<<N as Network>::PeerId>,
    block_hash: Blake2bHash,
}

impl<N: Network> BlockchainPushResult<N> {
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
pub async fn push_block_and_chunks<N: Network>(
    network: Arc<N>,
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    pubsub_id: Option<N::PubsubId>,
    block: Block,
    chunks: Vec<ChunkAndId<N>>,
) -> (
    Result<PushResult, PushError>,
    Result<ChunksPushResult, ChunksPushError>,
    Blake2bHash,
) {
    let push_results =
        spawn_blocking(move || blockchain_push(blockchain, bls_cache, Some(block), chunks))
            .await
            .expect("blockchain.push() should not panic");
    validate_message(network, pubsub_id, &push_results.block_push_result, true);

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
    pubsub_id: Option<N::PubsubId>,
    block: Block,
    include_body: bool,
) -> (Result<PushResult, PushError>, Blake2bHash) {
    let push_results =
        spawn_blocking(move || blockchain_push::<N>(blockchain, bls_cache, Some(block), vec![]))
            .await
            .expect("blockchain.push() should not panic");
    validate_message(
        network,
        pubsub_id,
        &push_results.block_push_result,
        include_body,
    );
    (
        push_results.block_push_result.unwrap(),
        push_results.block_hash,
    )
}

/// Pushes a sequence of blocks to the blockchain.
/// This case is different from pushing single blocks in a for loop,
/// because an invalid block automatically invalidates the remainder of the sequence.
pub async fn push_multiple_blocks_with_chunks<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    blocks: Vec<(Block, Vec<ChunkAndId<N>>)>,
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
    for (block, mut chunks) in block_iter.by_ref() {
        log::debug!("Pushing block {} from missing blocks response", block);

        let blockchain2 = blockchain.clone();
        let bls_cache2 = Arc::clone(&bls_cache);

        // If a previous chunk failed, we should discard all subsequent chunks.
        if push_chunk_result.is_err() {
            chunks.clear();
        }

        let push_results = spawn_blocking(move || {
            blockchain_push::<N>(blockchain2, bls_cache2, Some(block), chunks)
        })
        .await
        .expect("blockchain.push() should not panic");

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
    for (block, _) in block_iter {
        invalid_blocks.insert(block.hash());
    }
    (
        push_result,
        push_chunk_result,
        adopted_blocks,
        invalid_blocks,
    )
}

/// Pushes a sequence of blocks to the blockchain.
/// This case is different from pushing single blocks in a for loop,
/// because an invalid block automatically invalidates the remainder of the sequence.
pub async fn push_multiple_blocks<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    blocks: Vec<Block>,
) -> (
    Result<PushResult, PushError>,
    Vec<Blake2bHash>,
    HashSet<Blake2bHash>,
) {
    let blocks = blocks.into_iter().map(|block| (block, vec![])).collect();
    push_multiple_blocks_with_chunks::<N>(blockchain, bls_cache, blocks)
        .map(|(push_result, _, adopted_blocks, invalid_blocks)| {
            (push_result, adopted_blocks, invalid_blocks)
        })
        .await
}

/// Pushes the chunks to the current blockchain state.
pub async fn push_chunks_only<N: Network>(
    blockchain: BlockchainProxy,
    bls_cache: Arc<Mutex<PublicKeyCache>>,
    chunks: Vec<ChunkAndId<N>>,
) -> (Result<ChunksPushResult, ChunksPushError>, Blake2bHash) {
    let push_results = spawn_blocking(move || blockchain_push(blockchain, bls_cache, None, chunks))
        .await
        .expect("blockchain.push() should not panic");

    // TODO Ban peer depending on type of chunk error?

    (push_results.push_chunks_result, push_results.block_hash)
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
    chunks: Vec<ChunkAndId<N>>,
) -> BlockchainPushResult<N> {
    let (chunks, peer_ids): (Vec<_>, Vec<N::PeerId>) =
        chunks.into_iter().map(ChunkAndId::into_pair).unzip();

    // Push the block to the blockchain.
    let blockchain_push_result;
    if let Some(block) = block {
        let block_hash = block.hash();
        // Update validator keys from BLS public key cache.
        block.update_validator_keys(&mut bls_cache.lock());
        match blockchain {
            BlockchainProxy::Full(ref blockchain) => {
                // We push the block and if it fails we return immediately, without committing chunks.
                let push_result =
                    Blockchain::push_with_chunks(blockchain.upgradable_read(), block, chunks);

                blockchain_push_result =
                    BlockchainPushResult::with_block_result(push_result, block_hash, &peer_ids);
            }
            BlockchainProxy::Light(ref _blockchain) => {
                // TODO
                todo!()
            }
        }
    } else {
        let block_hash = blockchain.read().head_hash();

        match blockchain {
            BlockchainProxy::Full(ref blockchain) => {
                // We push the chunks.
                let chunks_push_result = blockchain.read().commit_chunks(chunks, &block_hash);
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

fn validate_message<N: Network>(
    network: Arc<N>,
    pubsub_id: Option<N::PubsubId>,
    block_push_result: &Option<Result<PushResult, PushError>>,
    include_body: bool,
) {
    if let Some(id) = pubsub_id {
        if let Some(ref push_result) = block_push_result {
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
            if include_body {
                network.validate_message::<BlockTopic>(id, acceptance);
            } else {
                network.validate_message::<BlockHeaderTopic>(id, acceptance);
            }
        }
    }
}
