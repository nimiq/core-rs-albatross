use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};

use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{Network, NetworkEvent, SubscribeEvents},
    request::RequestError,
};

use crate::messages::RequestMissingBlocks;
use crate::sync::history::HistorySyncReturn;
use crate::sync::sync_queue::SyncQueue;

pub trait RequestComponent<N: Network>: Stream<Item = RequestComponentEvent> + Unpin {
    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    );

    fn put_peer_into_sync_mode(&mut self, peer_id: N::PeerId);

    fn num_peers(&self) -> usize;

    fn peers(&self) -> Vec<N::PeerId>;
}

#[derive(Debug)]
pub enum RequestComponentEvent {
    ReceivedBlocks(Vec<Block>),
}

pub trait HistorySyncStream<T>: Stream<Item = HistorySyncReturn<T>> + Unpin + Send {
    fn add_peer(&self, peer_id: T);
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
    sync_method: Pin<Box<dyn HistorySyncStream<TNetwork::PeerId>>>,
    peers: HashSet<TNetwork::PeerId>, // this map holds the strong references to up-to-date peers
    outdated_peers: HashSet<TNetwork::PeerId>, //
    outdated_timeouts: HashMap<TNetwork::PeerId, Instant>,
    network_event_rx: SubscribeEvents<TNetwork::PeerId>,
}

impl<TNetwork: Network + 'static> BlockRequestComponent<TNetwork> {
    const NUM_PENDING_BLOCKS: usize = 5;

    const CHECK_OUTDATED_TIMEOUT: Duration = Duration::from_secs(20);

    pub fn new(
        sync_method: Pin<Box<dyn HistorySyncStream<TNetwork::PeerId>>>,
        network_event_rx: SubscribeEvents<TNetwork::PeerId>,
        network: Arc<TNetwork>,
    ) -> Self {
        Self {
            sync_method,
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
            outdated_peers: Default::default(),
            outdated_timeouts: Default::default(),
            network_event_rx,
        }
    }

    /// Adds all outdated peers that were checked more than TIMEOUT ago to history sync
    fn check_peers_up_to_date(&mut self) {
        let mut peers_todo = Vec::new();
        self.outdated_timeouts.retain(|&peer_id, last_checked| {
            let timeouted = last_checked.elapsed() >= Self::CHECK_OUTDATED_TIMEOUT;
            if timeouted {
                peers_todo.push(peer_id);
            }
            !timeouted
        });
        for peer_id in peers_todo {
            debug!("Adding outdated peer {:?} to history sync", peer_id);
            let peer_id = self.outdated_peers.take(&peer_id).unwrap();
            self.sync_method.add_peer(peer_id);
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
    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) {
        self.sync_queue.add_ids(vec![(target_block_hash, locators)]);
    }

    fn put_peer_into_sync_mode(&mut self, peer_id: TNetwork::PeerId) {
        // If the peer is not in `agents`, it's already in sync mode.
        if let Some(peer_id) = self.peers.take(&peer_id) {
            debug!("Putting peer back into sync mode: {:?}", peer_id);
            self.sync_method.add_peer(peer_id);
        }
    }

    fn num_peers(&self) -> usize {
        self.sync_queue.num_peers()
    }

    fn peers(&self) -> Vec<TNetwork::PeerId> {
        self.peers.iter().cloned().collect()
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

        // 2. Poll self.sync_method and add new peers to self.sync_queue.
        while let Poll::Ready(result) = self.sync_method.poll_next_unpin(cx) {
            match result {
                Some(HistorySyncReturn::Good(peer_id)) => {
                    debug!("Adding peer {:?} into follow mode", peer_id);
                    self.sync_queue.add_peer(peer_id);
                    self.peers.insert(peer_id);
                }
                Some(HistorySyncReturn::Outdated(peer_id)) => {
                    debug!(
                        "History sync returned outdated peer {:?}. Waiting.",
                        peer_id
                    );
                    self.outdated_timeouts.insert(peer_id, Instant::now());
                    self.outdated_peers.insert(peer_id);
                }
                None => {}
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

        self.check_peers_up_to_date();

        Poll::Pending
    }
}
