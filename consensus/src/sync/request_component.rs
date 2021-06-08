use crate::consensus_agent::ConsensusAgent;
use crate::sync::sync_queue::SyncQueue;
use block::Block;
use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};
use hash::Blake2bHash;
use network_interface::{network::NetworkEvent, peer::Peer};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use tokio_stream::wrappers::BroadcastStream;

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
    Stream<Item = Arc<ConsensusAgent<TPeer>>> + Unpin + Send
{
    fn add_peer(&self, peer: Arc<TPeer>);
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
pub struct BlockRequestComponent<TPeer: Peer> {
    sync_queue: SyncQueue<TPeer, (Blake2bHash, Vec<Blake2bHash>), Vec<Block>>, // requesting missing blocks from peers
    sync_method: Pin<Box<dyn HistorySyncStream<TPeer>>>,
    agents: HashMap<Arc<TPeer>, Arc<ConsensusAgent<TPeer>>>, // this map holds the strong references to connected peers
    network_event_rx: BroadcastStream<NetworkEvent<TPeer>>,
}

impl<TPeer: Peer + 'static> BlockRequestComponent<TPeer> {
    const NUM_PENDING_BLOCKS: usize = 5;

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
            network_event_rx,
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
        self.sync_method.add_peer(peer.clone());
        self.agents.remove(&peer);
    }

    fn num_peers(&self) -> usize {
        self.sync_queue.num_peers()
    }

    fn peers(&self) -> Vec<Weak<ConsensusAgent<TPeer>>> {
        self.agents
            .values()
            .map(|agent| Arc::downgrade(agent))
            .collect()
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
        if let Poll::Ready(Some(result)) = self.sync_method.poll_next_unpin(cx) {
            self.sync_queue.add_peer(Arc::downgrade(&result));
            self.agents.insert(Arc::clone(&result.peer), result);
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
