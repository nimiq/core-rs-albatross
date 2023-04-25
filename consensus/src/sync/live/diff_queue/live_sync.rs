use std::{collections::VecDeque, sync::Arc};

use futures::{
    future::{self, BoxFuture},
    FutureExt, Stream,
};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_network_interface::network::Network;
use parking_lot::Mutex;

use super::{DiffQueue, QueuedDiff};
use crate::sync::{
    live::{block_queue::live_sync::PushOpResult, queue, queue::LiveSyncQueue},
    syncer::{LiveSyncEvent, LiveSyncPeerEvent},
};

impl<N: Network> LiveSyncQueue<N> for DiffQueue<N> {
    type QueueResult = QueuedDiff<N>;
    type PushResult = PushOpResult<N>;

    fn push_queue_result(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        result: Self::QueueResult,
        _include_body: bool,
    ) -> VecDeque<BoxFuture<'static, Self::PushResult>> {
        let mut future_results = VecDeque::new();
        match result {
            QueuedDiff::Head((block, pubsub_id), diff) => {
                // Push block.
                future_results.push_back(
                    queue::push_block_and_chunks(
                        network,
                        blockchain,
                        bls_cache,
                        pubsub_id,
                        block,
                        diff,
                        vec![],
                    )
                    .map(|(push_result, _, hash)| PushOpResult::Head(push_result, hash))
                    .boxed(),
                );
            }
            QueuedDiff::Buffered(buffered_blocks) => {
                for ((block, pubsub_id), diff) in buffered_blocks {
                    let res = queue::push_block_and_chunks(
                        Arc::clone(&network),
                        blockchain.clone(),
                        Arc::clone(&bls_cache),
                        pubsub_id,
                        block,
                        diff,
                        vec![],
                    )
                    .map(|(push_result, _, hash)| PushOpResult::Buffered(push_result, hash))
                    .boxed();
                    future_results.push_back(res);
                }
            }
            QueuedDiff::Missing(blocks) => {
                let blocks = blocks
                    .into_iter()
                    .map(|(block, diff)| (block, diff, vec![]))
                    .collect();
                // Pushes multiple blocks.
                future_results.push_back(
                    queue::push_multiple_blocks_with_chunks::<N>(blockchain, bls_cache, blocks)
                        .map(|(push_result, _, adopted_blocks, invalid_blocks)| {
                            PushOpResult::Missing(push_result, adopted_blocks, invalid_blocks)
                        })
                        .boxed(),
                );
            }
            QueuedDiff::TooFarAhead(peer_id) => {
                // Peer is too far ahead.
                future_results.push_back(
                    future::ready(PushOpResult::PeerEvent(LiveSyncPeerEvent::Ahead(peer_id)))
                        .boxed(),
                );
            }
            QueuedDiff::PeerIncompleteState(peer_id) | QueuedDiff::TooFarBehind(peer_id) => {
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
        self.block_queue.process_push_result(item)
    }

    fn num_peers(&self) -> usize {
        self.block_queue.num_peers()
    }

    fn include_micro_bodies(&self) -> bool {
        true
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.block_queue.peers()
    }

    fn add_peer(&self, peer_id: N::PeerId) {
        self.block_queue.add_peer(peer_id)
    }

    /// Adds an additional block stream by replacing the current block stream with a `select` of both streams.
    fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = (Block, N::PeerId, Option<N::PubsubId>)> + Send + 'static,
    {
        self.block_queue.add_block_stream(block_stream)
    }
}
