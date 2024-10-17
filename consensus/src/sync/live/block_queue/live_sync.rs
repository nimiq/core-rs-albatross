use std::{
    collections::{HashSet, VecDeque},
    mem,
    sync::Arc,
};

use futures::{
    future::{self, BoxFuture},
    stream::{empty, select},
    FutureExt, Stream, StreamExt,
};
use nimiq_blockchain_interface::{PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use nimiq_utils::WakerExt;
use parking_lot::Mutex;

use super::{BlockAndSource, QueuedBlock};
use crate::sync::{
    live::{
        block_queue::queue::BlockQueue,
        queue::{self, LiveSyncQueue},
    },
    syncer::{LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent},
};

pub enum PushOpResult<N: Network> {
    Head(Result<PushResult, PushError>, u32, Blake2bHash),
    Buffered(Result<PushResult, PushError>, u32, Blake2bHash),
    Missing(
        Result<PushResult, PushError>,
        Vec<(u32, Blake2bHash)>,
        HashSet<Blake2bHash>,
    ),
    PeerEvent(LiveSyncPeerEvent<N::PeerId>),
}

impl<N: Network> LiveSyncQueue<N> for BlockQueue<N> {
    type QueueResult = QueuedBlock<N>;
    type PushResult = PushOpResult<N>;

    fn push_queue_result(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        result: Self::QueueResult,
    ) -> VecDeque<BoxFuture<'static, Self::PushResult>> {
        let mut future_results = VecDeque::new();
        match result {
            QueuedBlock::Head((block, block_source)) => {
                // Push block.
                future_results.push_back(
                    queue::push_block_only(network, blockchain, bls_cache, block, block_source)
                        .map(|(push_result, block_height, hash)| {
                            PushOpResult::Head(push_result, block_height, hash)
                        })
                        .boxed(),
                );
            }
            QueuedBlock::Buffered(buffered_blocks) => {
                for (block, block_source) in buffered_blocks {
                    let res = queue::push_block_only(
                        Arc::clone(&network),
                        blockchain.clone(),
                        Arc::clone(&bls_cache),
                        block,
                        block_source,
                    )
                    .map(|(push_result, block_height, hash)| {
                        PushOpResult::Buffered(push_result, block_height, hash)
                    })
                    .boxed();
                    future_results.push_back(res);
                }
            }
            QueuedBlock::Missing(blocks) => {
                // Pushes multiple blocks.
                future_results.push_back(
                    queue::push_multiple_blocks::<N>(blockchain, bls_cache, blocks)
                        .map(|(push_result, adopted_blocks, invalid_blocks)| {
                            PushOpResult::Missing(push_result, adopted_blocks, invalid_blocks)
                        })
                        .boxed(),
                );
            }
            QueuedBlock::TooFarAhead(peer_id) => {
                // Peer is too far ahead.
                future_results.push_back(
                    future::ready(PushOpResult::PeerEvent(LiveSyncPeerEvent::Ahead(peer_id)))
                        .boxed(),
                );
            }
            QueuedBlock::TooFarBehind(peer_id) => {
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
        match item {
            PushOpResult::Head(Ok(result), block_height, hash) => {
                self.on_block_processed(block_height, &hash, true);
                if result == PushResult::Extended || result == PushResult::Rebranched {
                    return Some(LiveSyncEvent::PushEvent(
                        LiveSyncPushEvent::AcceptedAnnouncedBlock(hash),
                    ));
                }
            }
            PushOpResult::Buffered(Ok(result), block_height, hash) => {
                self.on_block_processed(block_height, &hash, true);
                if result == PushResult::Extended || result == PushResult::Rebranched {
                    return Some(LiveSyncEvent::PushEvent(
                        LiveSyncPushEvent::AcceptedBufferedBlock(hash, self.num_buffered_blocks()),
                    ));
                }
            }
            PushOpResult::Missing(result, adopted_blocks, mut invalid_blocks) => {
                for (block_height, hash) in &adopted_blocks {
                    self.on_block_processed(*block_height, hash, true);
                }
                for hash in &invalid_blocks {
                    self.on_block_processed(0, hash, false);
                }

                self.remove_invalid_blocks(&mut invalid_blocks);

                if result.is_ok() && !adopted_blocks.is_empty() {
                    return Some(LiveSyncEvent::PushEvent(
                        LiveSyncPushEvent::ReceivedMissingBlocks(
                            adopted_blocks
                                .into_iter()
                                .map(|(_, block_hash)| block_hash)
                                .collect(),
                        ),
                    ));
                }
            }
            PushOpResult::Head(Err(result), block_height, hash)
            | PushOpResult::Buffered(Err(result), block_height, hash) => {
                // If there was a blockchain push error, we remove the block from the pending blocks
                log::trace!("Head push operation failed because of {}", result);
                self.on_block_processed(block_height, &hash, false);
                return Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::RejectedBlock(
                    hash,
                )));
            }
            PushOpResult::PeerEvent(event) => return Some(LiveSyncEvent::PeerEvent(event)),
        };
        None
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.request_component.peers()
    }

    fn num_peers(&self) -> usize {
        self.request_component.num_peers()
    }

    fn add_peer(&self, peer_id: N::PeerId) {
        self.request_component.add_peer(peer_id)
    }

    /// Adds a block stream by replacing the current block stream with a `select` of both streams.
    fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = BlockAndSource<N>> + Send + 'static,
    {
        // We need to safely remove the old block stream first.
        let prev_block_stream = mem::replace(&mut self.block_stream, empty().boxed());
        self.block_stream = select(prev_block_stream, block_stream).boxed();
        self.waker.wake();
    }

    fn include_body(&self) -> bool {
        self.config.include_body
    }

    fn resolve_block(&mut self, request: crate::consensus::ResolveBlockRequest<N>) {
        BlockQueue::resolve_block(self, request)
    }

    fn acceptance_window_size(&self) -> u32 {
        self.config.window_ahead_max
    }
}
