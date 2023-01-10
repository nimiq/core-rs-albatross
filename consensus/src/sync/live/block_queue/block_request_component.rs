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
use parking_lot::RwLock;

use crate::{
    messages::RequestMissingBlocks,
    sync::{peer_list::PeerList, sync_queue::SyncQueue},
};

#[derive(Debug)]
pub enum BlockRequestComponentEvent {
    ReceivedBlocks(Vec<Block>),
}

/// Peer Tracking & Block Request Component.
/// We use this component to request missing blocks from peers.
///
/// This component has:
///
/// - The sync queue which manages the requests and responses.
/// - The peers list.
/// - The network stream of events used to remove the peers that have left.  
/// - Weather we include the body of a block.
///
/// The public interface allows to request blocks, which are not immediately returned.
/// The blocks instead are returned by polling the component.
pub struct BlockRequestComponent<N: Network> {
    sync_queue: SyncQueue<N, (Blake2bHash, Vec<Blake2bHash>, bool), Vec<Block>>, // requesting missing blocks from peers
    pub(crate) peers: Arc<RwLock<PeerList<N>>>,
    network_event_rx: SubscribeEvents<N::PeerId>,
    include_micro_bodies: bool,
}

impl<N: Network> BlockRequestComponent<N> {
    const NUM_PENDING_BLOCKS: usize = 5;

    pub fn new(
        network_event_rx: SubscribeEvents<N::PeerId>,
        network: Arc<N>,
        include_micro_bodies: bool,
    ) -> Self {
        let peers = Arc::new(RwLock::new(PeerList::default()));
        Self {
            sync_queue: SyncQueue::new(
                network,
                vec![],
                Arc::clone(&peers),
                Self::NUM_PENDING_BLOCKS,
                |(target_block_hash, locators, include_micro_bodies), network, peer_id| {
                    async move {
                        let res = Self::request_missing_blocks_from_peer(
                            network,
                            peer_id,
                            target_block_hash,
                            locators,
                            include_micro_bodies,
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
            peers,
            network_event_rx,
            include_micro_bodies,
        }
    }

    async fn request_missing_blocks_from_peer(
        network: Arc<N>,
        peer_id: N::PeerId,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
        include_micro_bodies: bool,
    ) -> Result<Option<Vec<Block>>, RequestError> {
        network
            .request::<RequestMissingBlocks>(
                RequestMissingBlocks {
                    locators,
                    target_hash: target_block_hash,
                    include_micro_bodies,
                },
                peer_id,
            )
            .await
            .map(|response| response.blocks)
    }

    pub fn add_peer(&self, peer_id: N::PeerId) {
        self.peers.write().add_peer(peer_id);
    }

    pub fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) {
        self.sync_queue.add_ids(vec![(
            target_block_hash,
            locators,
            self.include_micro_bodies,
        )]);
    }

    pub fn num_peers(&self) -> usize {
        self.peers.read().len()
    }

    pub fn peers(&self) -> Vec<N::PeerId> {
        self.peers.read().peers().clone()
    }

    pub fn take_peer(&self, peer_id: &N::PeerId) -> Option<N::PeerId> {
        if self.peers.write().remove_peer(peer_id) {
            return Some(*peer_id);
        }
        None
    }

    pub fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        Arc::clone(&self.peers)
    }
}

impl<N: Network> Stream for BlockRequestComponent<N> {
    type Item = BlockRequestComponentEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // 1. Poll network events to remove peers.
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            if let Ok(NetworkEvent::PeerLeft(peer_id)) = result {
                // Remove peers that left.
                self.peers.write().remove_peer(&peer_id);
            }
        }

        // 3. Poll self.sync_queue, return results.
        while let Poll::Ready(Some(result)) = self.sync_queue.poll_next_unpin(cx) {
            match result {
                Ok(blocks) => {
                    return Poll::Ready(Some(BlockRequestComponentEvent::ReceivedBlocks(blocks)))
                }
                Err((target_hash, _, _)) => {
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
