use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;

use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::NetworkEvent, peer::Peer};

use crate::consensus_agent::ConsensusAgent;
use crate::sync::history::HistorySyncReturn;
use crate::sync::sync_queue::SyncQueue;

pub trait RequestComponent<P: Peer>: Stream<Item = RequestComponentEvent> + Unpin {
    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    );

    fn put_peer_into_sync_mode(&mut self, peer: Arc<P>);

    fn num_peers(&self) -> usize;

    fn peers(&self) -> Vec<Weak<ConsensusAgent<P>>>;
}

#[derive(Debug)]
pub enum RequestComponentEvent {
    ReceivedBlocks(Vec<Block>),
}

pub trait HistorySyncStream<TPeer: Peer>:
    Stream<Item = HistorySyncReturn<TPeer>> + Unpin + Send
{
    fn add_agent(&self, agent: Arc<ConsensusAgent<TPeer>>);
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
pub struct BlockRequestComponent<TPeer: Peer + 'static> {
    sync_queue: SyncQueue<TPeer, (Blake2bHash, Vec<Blake2bHash>), Vec<Block>>, // requesting missing blocks from peers
    sync_method: Pin<Box<dyn HistorySyncStream<TPeer>>>,
    agents: HashMap<Arc<TPeer>, Arc<ConsensusAgent<TPeer>>>, // this map holds the strong references to up-to-date peers
    outdated_agents: HashMap<Arc<TPeer>, Arc<ConsensusAgent<TPeer>>>, //
    outdated_timeouts: HashMap<Arc<TPeer>, Instant>,
    network_event_rx: BroadcastStream<NetworkEvent<TPeer>>,
}

impl<TPeer: Peer + 'static> BlockRequestComponent<TPeer> {
    const NUM_PENDING_BLOCKS: usize = 5;

    const CHECK_OUTDATED_TIMEOUT: Duration = Duration::from_secs(20);

    pub fn new(
        sync_method: Pin<Box<dyn HistorySyncStream<TPeer>>>,
        network_event_rx: BroadcastStream<NetworkEvent<TPeer>>,
    ) -> Self {
        Self {
            sync_method,
            sync_queue: SyncQueue::new(
                vec![],
                vec![],
                Self::NUM_PENDING_BLOCKS,
                |(target_block_hash, locators), peer| {
                    async move {
                        let res = peer
                            .request_missing_blocks(target_block_hash, locators)
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
            agents: Default::default(),
            outdated_agents: Default::default(),
            outdated_timeouts: Default::default(),
            network_event_rx,
        }
    }

    /// Adds all outdated peers that were checked more than TIMEOUT ago to history sync
    fn check_peers_up_to_date(&mut self) {
        let peers_todo = self
            .outdated_timeouts
            .drain_filter(|_, last_checked| last_checked.elapsed() >= Self::CHECK_OUTDATED_TIMEOUT);
        for (peer, _) in peers_todo {
            debug!("Adding outdated peer {:?} to history sync", peer.id());
            let agent = self.outdated_agents.remove(&peer).unwrap();
            self.sync_method.add_agent(agent);
        }
    }
}

impl<TPeer: 'static + Peer> RequestComponent<TPeer> for BlockRequestComponent<TPeer> {
    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) {
        self.sync_queue.add_ids(vec![(target_block_hash, locators)]);
    }

    fn put_peer_into_sync_mode(&mut self, peer: Arc<TPeer>) {
        // If the peer is not in `agents`, it's already in sync mode.
        if let Some(agent) = self.agents.remove(&peer) {
            debug!("Putting peer back into sync mode: {:?}", peer.id());
            self.sync_method.add_agent(agent);
        }
    }

    fn num_peers(&self) -> usize {
        self.sync_queue.num_peers()
    }

    fn peers(&self) -> Vec<Weak<ConsensusAgent<TPeer>>> {
        self.agents.values().map(Arc::downgrade).collect()
    }
}

impl<TPeer: Peer + 'static> Stream for BlockRequestComponent<TPeer> {
    type Item = RequestComponentEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // 1. Poll network events to remove peers.
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            if let Ok(NetworkEvent::PeerLeft(peer)) = result {
                // Remove peers that left.
                self.agents.remove(&peer);
            }
        }

        // 2. Poll self.sync_method and add new peers to self.sync_queue.
        while let Poll::Ready(result) = self.sync_method.poll_next_unpin(cx) {
            match result {
                Some(HistorySyncReturn::Good(peer)) => {
                    debug!("Adding peer {:?} into follow mode", peer.peer.id());
                    self.sync_queue.add_peer(Arc::downgrade(&peer));
                    self.agents.insert(Arc::clone(&peer.peer), peer);
                }
                Some(HistorySyncReturn::Outdated(peer)) => {
                    debug!(
                        "History sync returned outdated peer {:?}. Waiting.",
                        peer.peer.id()
                    );
                    self.outdated_timeouts
                        .insert(Arc::clone(&peer.peer), Instant::now());
                    self.outdated_agents.insert(Arc::clone(&peer.peer), peer);
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
