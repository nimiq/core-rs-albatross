use std::{
    collections::{HashSet, VecDeque},
    mem,
    sync::Arc,
};

use futures::{
    future::{self, BoxFuture},
    FutureExt, Stream,
};
use futures::{
    stream::{empty, select},
    StreamExt,
};
use nimiq_block::Block;
use nimiq_blockchain_interface::{PushError, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use parking_lot::Mutex;

use super::block_request_component::RequestComponent;
use super::{BlockQueue, QueuedBlock};
use crate::sync::{
    live::queue::{self, LiveSyncQueue},
    syncer::{LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent},
};

pub enum PushOpResult<N: Network> {
    Head(Result<PushResult, PushError>, Blake2bHash),
    Buffered(Result<PushResult, PushError>, Blake2bHash),
    Missing(
        Result<PushResult, PushError>,
        Vec<Blake2bHash>,
        HashSet<Blake2bHash>,
    ),
    PeerEvent(LiveSyncPeerEvent<N::PeerId>),
}

impl<N: Network, TReq: RequestComponent<N>> LiveSyncQueue<N> for BlockQueue<N, TReq> {
    type QueueResult = QueuedBlock<N>;
    type PushResult = PushOpResult<N>;

    fn push_queue_result(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        result: Self::QueueResult,
        include_body: bool,
    ) -> VecDeque<BoxFuture<'static, Self::PushResult>> {
        let mut future_results = VecDeque::new();
        match result {
            QueuedBlock::Head((block, pubsub_id)) => {
                // Push block.
                future_results.push_back(
                    queue::push_block_only(
                        network,
                        blockchain,
                        bls_cache,
                        pubsub_id,
                        block,
                        include_body,
                    )
                    .map(|(push_result, hash)| PushOpResult::Head(push_result, hash))
                    .boxed(),
                );
            }
            QueuedBlock::Buffered(buffered_blocks) => {
                for (block, pubsub_id) in buffered_blocks {
                    let res = queue::push_block_only(
                        Arc::clone(&network),
                        blockchain.clone(),
                        Arc::clone(&bls_cache),
                        pubsub_id,
                        block,
                        include_body,
                    )
                    .map(|(push_result, hash)| PushOpResult::Buffered(push_result, hash))
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
            QueuedBlock::TooFarFuture(_, peer_id) => {
                // Peer is too far ahead.
                future_results.push_back(
                    future::ready(PushOpResult::PeerEvent(LiveSyncPeerEvent::AdvancedPeer(
                        peer_id,
                    )))
                    .boxed(),
                );
            }
            QueuedBlock::TooDistantPast(_, peer_id) => {
                // Peer is too far behind.
                future_results.push_back(
                    future::ready(PushOpResult::PeerEvent(LiveSyncPeerEvent::OutdatedPeer(
                        peer_id,
                    )))
                    .boxed(),
                );
            }
        }
        future_results
    }

    fn process_push_result(&mut self, item: Self::PushResult) -> Option<LiveSyncEvent<N::PeerId>> {
        match item {
            PushOpResult::Head(Ok(result), hash) => {
                self.on_block_processed(&hash);
                if result == PushResult::Extended || result == PushResult::Rebranched {
                    return Some(LiveSyncEvent::PushEvent(
                        LiveSyncPushEvent::AcceptedAnnouncedBlock(hash),
                    ));
                }
            }
            PushOpResult::Buffered(Ok(result), hash) => {
                self.on_block_processed(&hash);
                if result == PushResult::Extended || result == PushResult::Rebranched {
                    return Some(LiveSyncEvent::PushEvent(
                        LiveSyncPushEvent::AcceptedBufferedBlock(hash, self.buffered_blocks_len()),
                    ));
                }
            }
            PushOpResult::Missing(result, mut adopted_blocks, mut invalid_blocks) => {
                for hash in &adopted_blocks {
                    self.on_block_processed(hash);
                }
                for hash in &invalid_blocks {
                    self.on_block_processed(hash);
                }

                self.remove_invalid_blocks(&mut invalid_blocks);

                if result.is_ok() && !adopted_blocks.is_empty() {
                    let hash = adopted_blocks.pop().expect("adopted_blocks not empty");
                    return Some(LiveSyncEvent::PushEvent(
                        LiveSyncPushEvent::ReceivedMissingBlocks(hash, adopted_blocks.len() + 1),
                    ));
                }
            }
            PushOpResult::Head(Err(result), hash) | PushOpResult::Buffered(Err(result), hash) => {
                // If there was a blockchain push error, we remove the block from the pending blocks
                log::trace!("Head push operation failed because of {}", result);
                self.on_block_processed(&hash);
                return Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::RejectedBlock(
                    hash,
                )));
            }
            PushOpResult::PeerEvent(event) => return Some(LiveSyncEvent::PeerEvent(event)),
        };
        None
    }

    fn num_peers(&self) -> usize {
        self.request_component.num_peers()
    }

    fn include_micro_bodies(&self) -> bool {
        self.config.include_micro_bodies
    }

    fn peers(&self) -> Vec<N::PeerId> {
        self.request_component.peers()
    }

    fn add_peer(&self, peer_id: N::PeerId) {
        self.request_component.add_peer(peer_id)
    }

    /// Adds an additional block stream by replacing the current block stream with a `select` of both streams.
    fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = (Block, N::PeerId, Option<N::PubsubId>)> + Send + 'static,
    {
        // We need to safely remove the old block stream first.
        let prev_block_stream = mem::replace(&mut self.block_stream, empty().boxed());
        self.block_stream = select(prev_block_stream, block_stream).boxed();
    }
}
