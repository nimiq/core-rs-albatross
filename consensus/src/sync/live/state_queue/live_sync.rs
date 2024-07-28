use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

use futures::{
    future::{self, BoxFuture},
    FutureExt, Stream,
};
use nimiq_blockchain_interface::{ChunksPushError, ChunksPushResult, PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use parking_lot::Mutex;

use super::{ChunkAndSource, QueuedStateChunks, StateQueue};
use crate::{
    consensus::ResolveBlockRequest,
    sync::{
        live::{
            block_queue::{live_sync::PushOpResult as BlockPushOpResult, BlockAndSource},
            queue::{self, LiveSyncQueue},
        },
        syncer::{LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent},
    },
};

pub enum PushOpResult<N: Network> {
    Head(
        Result<PushResult, PushError>,
        Result<ChunksPushResult, ChunksPushError>,
        Blake2bHash,
    ),
    HeadChunk(Result<ChunksPushResult, ChunksPushError>, Blake2bHash),
    Buffered(
        Result<PushResult, PushError>,
        Result<ChunksPushResult, ChunksPushError>,
        Blake2bHash,
    ),
    Missing(
        Result<PushResult, PushError>,
        Result<ChunksPushResult, ChunksPushError>,
        Vec<Blake2bHash>,
        HashSet<Blake2bHash>,
    ),
    PeerEvent(LiveSyncPeerEvent<N::PeerId>),
}

impl<N: Network> PushOpResult<N> {
    pub fn get_push_chunk_error(&self) -> Option<&ChunksPushError> {
        match self {
            PushOpResult::Head(_, Err(e), _)
            | PushOpResult::HeadChunk(Err(e), _)
            | PushOpResult::Buffered(_, Err(e), _)
            | PushOpResult::Missing(_, Err(e), _, _) => Some(e),
            _ => None,
        }
    }

    pub fn get_push_block_error(&self) -> Option<&PushError> {
        match self {
            PushOpResult::Head(Err(push_error), _, _)
            | PushOpResult::Buffered(Err(push_error), _, _)
            | PushOpResult::Missing(Err(push_error), _, _, _) => Some(push_error),
            _ => None,
        }
    }

    pub fn ignored_all_chunks(&self) -> bool {
        match self {
            PushOpResult::Head(_, Ok(ChunksPushResult::Chunks(committed, ignored)), _)
            | PushOpResult::HeadChunk(Ok(ChunksPushResult::Chunks(committed, ignored)), _)
            | PushOpResult::Buffered(_, Ok(ChunksPushResult::Chunks(committed, ignored)), _)
            | PushOpResult::Missing(_, Ok(ChunksPushResult::Chunks(committed, ignored)), _, _) => {
                *committed == 0 && *ignored > 0
            }
            _ => false,
        }
    }

    fn into_block_push_result(self) -> Option<BlockPushOpResult<N>> {
        match self {
            PushOpResult::Head(push_result, _, block_hash) => {
                Some(BlockPushOpResult::Head(push_result, block_hash))
            }
            PushOpResult::HeadChunk(_, _) => None,
            PushOpResult::Buffered(push_result, _, block_hash) => {
                Some(BlockPushOpResult::Buffered(push_result, block_hash))
            }
            PushOpResult::Missing(push_result, _, adopted_blocks, invalid_blocks) => Some(
                BlockPushOpResult::Missing(push_result, adopted_blocks, invalid_blocks),
            ),
            PushOpResult::PeerEvent(event) => Some(BlockPushOpResult::PeerEvent(event)),
        }
    }
}

impl<N: Network> LiveSyncQueue<N> for StateQueue<N> {
    type QueueResult = QueuedStateChunks<N>;
    type PushResult = PushOpResult<N>;

    fn push_queue_result(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        result: Self::QueueResult,
    ) -> VecDeque<BoxFuture<'static, Self::PushResult>> {
        let mut future_results = VecDeque::new();
        match result {
            QueuedStateChunks::Head((block, block_source), diff, chunks) => {
                // Push block.
                future_results.push_back(
                    queue::push_block_and_chunks(
                        network,
                        blockchain,
                        bls_cache,
                        block,
                        block_source,
                        diff,
                        chunks,
                    )
                    .map(|(push_result, push_chunk_error, hash)| {
                        PushOpResult::Head(push_result, push_chunk_error, hash)
                    })
                    .boxed(),
                );
            }
            QueuedStateChunks::Buffered(buffered_blocks) => {
                for ((block, block_source), diff, chunks) in buffered_blocks {
                    let res = queue::push_block_and_chunks(
                        Arc::clone(&network),
                        blockchain.clone(),
                        Arc::clone(&bls_cache),
                        block,
                        block_source,
                        diff,
                        chunks,
                    )
                    .map(|(push_result, push_chunk_error, hash)| {
                        PushOpResult::Buffered(push_result, push_chunk_error, hash)
                    })
                    .boxed();
                    future_results.push_back(res);
                }
            }
            QueuedStateChunks::Missing(blocks) => {
                // Pushes multiple blocks.
                future_results.push_back(
                    queue::push_multiple_blocks_with_chunks::<N>(blockchain, bls_cache, blocks)
                        .map(
                            |(push_result, push_chunk_error, adopted_blocks, invalid_blocks)| {
                                PushOpResult::Missing(
                                    push_result,
                                    push_chunk_error,
                                    adopted_blocks,
                                    invalid_blocks,
                                )
                            },
                        )
                        .boxed(),
                );
            }
            QueuedStateChunks::HeadStateChunk(chunks) => {
                // Chunks only.
                future_results.push_back(
                    queue::push_chunks_only::<N>(blockchain, bls_cache, chunks)
                        .map(|(push_chunk_error, block_hash)| {
                            PushOpResult::HeadChunk(push_chunk_error, block_hash)
                        })
                        .boxed(),
                );
            }
            QueuedStateChunks::TooFarFutureBlock(peer_id)
            | QueuedStateChunks::TooFarFutureChunk(ChunkAndSource { peer_id, .. }) => {
                // Peer is too far ahead.
                future_results.push_back(
                    future::ready(PushOpResult::PeerEvent(LiveSyncPeerEvent::Ahead(peer_id)))
                        .boxed(),
                );
            }
            QueuedStateChunks::PeerIncompleteState(peer_id)
            | QueuedStateChunks::TooDistantPastBlock(peer_id)
            | QueuedStateChunks::TooDistantPastChunk(ChunkAndSource { peer_id, .. }) => {
                // Peer is too far behind.
                future_results.push_back(
                    future::ready(PushOpResult::PeerEvent(LiveSyncPeerEvent::Behind(peer_id)))
                        .boxed(),
                );
            }
        }
        future_results
    }

    fn process_push_result(&mut self, item: Self::PushResult) -> Option<LiveSyncEvent<N::PeerId>> {
        // PITODO Avoid resetting the chunk request chain if a block error occurred on a block
        // that did not have any chunks in the current chunk chain.

        // Resets the chain of chunks when encountering an error or ignoring all chunks.
        // In all these cases, subsequent chunks are likely not to match our state.
        if item.get_push_chunk_error().is_some()
            || item.get_push_block_error().is_some()
            || item.ignored_all_chunks()
        {
            self.reset_chunk_request_chain();
        }

        match item {
            PushOpResult::HeadChunk(Ok(ChunksPushResult::Chunks(committed, _)), block_hash)
                if committed > 0 =>
            {
                // If we accepted chunks for the head block without error, we emit an event.
                // If there was an error, we do not know for sure whether or not a chunk was accepted.
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    block_hash,
                )))
            }
            PushOpResult::Missing(
                result,
                push_chunks_result,
                adopted_blocks,
                mut invalid_blocks,
            ) => {
                self.remove_invalid_blocks(&mut invalid_blocks);

                self.diff_queue.process_push_result(
                    PushOpResult::Missing(
                        result,
                        push_chunks_result,
                        adopted_blocks,
                        invalid_blocks,
                    )
                    .into_block_push_result()?,
                )
            }
            item => self
                .diff_queue
                .process_push_result(item.into_block_push_result()?),
        }
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.diff_queue.peers()
    }

    fn num_peers(&self) -> usize {
        self.diff_queue.num_peers()
    }

    fn add_peer(&self, peer_id: N::PeerId) {
        self.diff_queue.add_peer(peer_id)
    }

    /// Adds a block stream by replacing the current block stream with a `select` of both streams.
    fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = BlockAndSource<N>> + Send + 'static,
    {
        self.diff_queue.add_block_stream(block_stream)
    }

    fn include_body(&self) -> bool {
        true
    }

    fn state_complete(&self) -> bool {
        self.start_key.is_complete()
    }

    fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        self.diff_queue.resolve_block(request)
    }
}
