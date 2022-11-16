use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{FutureExt, Stream, StreamExt};

use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{Network, NetworkEvent, SubscribeEvents},
    request::RequestError,
};

use crate::messages::RequestMissingBlocks;
use crate::sync::sync_queue::SyncQueue;

pub trait RequestComponent<N: Network>:
    Stream<Item = RequestComponentEvent> + Unpin + Send
{
    fn add_peer(&mut self, peer_id: N::PeerId);

    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    );

    fn num_peers(&self) -> usize;

    fn peers(&self) -> Vec<N::PeerId>;

    fn take_peer(&mut self, peer_id: &N::PeerId) -> Option<N::PeerId>;
}

#[derive(Debug)]
pub enum RequestComponentEvent {
    ReceivedBlocks(Vec<Block>),
}

/// Peer Tracking & Request Component
///
/// - Has sync queue
/// - Polls synced peers from history sync
/// - Puts peers to sync queue
/// - Removal happens automatically by the SyncQueue
///
/// Outside has a request blocks method, which doesn’t return the blocks.
/// The blocks instead are returned by polling the component.
pub struct BlockRequestComponent<TNetwork: Network + 'static> {
    sync_queue: SyncQueue<TNetwork, (Blake2bHash, Vec<Blake2bHash>), Vec<Block>>, // requesting missing blocks from peers
    peers: HashSet<TNetwork::PeerId>, // this map holds the strong references to up-to-date peers
    network_event_rx: SubscribeEvents<TNetwork::PeerId>,
}

impl<TNetwork: Network + 'static> BlockRequestComponent<TNetwork> {
    const NUM_PENDING_BLOCKS: usize = 5;

    pub fn new(
        network_event_rx: SubscribeEvents<TNetwork::PeerId>,
        network: Arc<TNetwork>,
    ) -> Self {
        Self {
            sync_queue: SyncQueue::new(
                network,
                vec![],
                vec![],
                Self::NUM_PENDING_BLOCKS,
                |(target_block_hash, locators), network, peer_id| {
                    async move {
                        let res = Self::request_missing_blocks_from_peer(
                            network,
                            peer_id,
                            target_block_hash,
                            locators,
                        )
                        .await;
                        if let Ok(Some(missing_blocks)) = res {
                            Some(missing_blocks)
                        } else {
                            None
                        }
                    }
                    .boxed()
                },
            ),
            peers: Default::default(),
            network_event_rx,
        }
    }

    pub async fn request_missing_blocks_from_peer(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) -> Result<Option<Vec<Block>>, RequestError> {
        network
            .request::<RequestMissingBlocks>(
                RequestMissingBlocks {
                    locators,
                    target_hash: target_block_hash,
                },
                peer_id,
            )
            .await
            .map(|response| response.blocks)
    }
}

impl<TNetwork: 'static + Network> RequestComponent<TNetwork> for BlockRequestComponent<TNetwork> {
    fn add_peer(&mut self, peer_id: TNetwork::PeerId) {
        self.peers.insert(peer_id);
        self.sync_queue.add_peer(peer_id);
    }

    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) {
        self.sync_queue.add_ids(vec![(target_block_hash, locators)]);
    }

    fn num_peers(&self) -> usize {
        self.peers.len()
    }

    fn peers(&self) -> Vec<TNetwork::PeerId> {
        self.peers.iter().cloned().collect()
    }

    fn take_peer(&mut self, peer_id: &TNetwork::PeerId) -> Option<TNetwork::PeerId> {
        self.peers.take(peer_id)
    }
}

impl<TNetwork: Network + 'static> Stream for BlockRequestComponent<TNetwork> {
    type Item = RequestComponentEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // 1. Poll network events to remove peers.
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            if let Ok(NetworkEvent::PeerLeft(peer_id)) = result {
                // Remove peers that left.
                self.peers.remove(&peer_id);
            }
        }

        // 3. Poll self.sync_queue, return results.
        while let Poll::Ready(Some(result)) = self.sync_queue.poll_next_unpin(cx) {
            match result {
                Ok(blocks) => {
                    return Poll::Ready(Some(RequestComponentEvent::ReceivedBlocks(blocks)))
                }
                Err((target_hash, _)) => {
                    debug!(
                        "Failed to retrieve missing blocks for target hash {}",
                        target_hash
                    );
                    // TODO: Do we need to do anything else?
                }
            }
        }

        Poll::Pending
    }
}
