use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt};
use nimiq_network_interface::network::{Network, NetworkEvent, SubscribeEvents};
use parking_lot::RwLock;

use crate::sync::{peer_list::PeerList, sync_queue::SyncQueue};

use super::state_queue::{RequestChunk, ResponseChunk};

pub struct ChunkRequestComponent<N: Network> {
    sync_queue: SyncQueue<N, RequestChunk, (ResponseChunk, N::PeerId)>,
    // These peers will be shared across the block request component and this component.
    peers: Arc<RwLock<PeerList<N>>>,
    network_event_rx: SubscribeEvents<N::PeerId>,
}

impl<N: Network> ChunkRequestComponent<N> {
    pub fn add_peer(&mut self, peer_id: N::PeerId) -> bool {
        self.peers.write().add_peer(peer_id)
    }

    pub fn remove_peer(&mut self, peer_id: &N::PeerId) {
        self.peers.write().remove_peer(peer_id);
    }

    pub fn num_peers(&self) -> usize {
        self.peers.read().len()
    }

    pub fn len(&self) -> usize {
        self.peers.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn has_pending_requests(&self) -> bool {
        !self.sync_queue.is_empty()
    }

    pub fn request_chunk(&mut self, request: RequestChunk) {
        self.sync_queue.add_ids(vec![request]);
    }
}

impl<N: Network> Stream for ChunkRequestComponent<N> {
    type Item = (ResponseChunk, N::PeerId);

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
                Ok((chunk, peer_id)) => return Poll::Ready(Some((chunk, peer_id))),
                Err(req) => {
                    debug!(
                        "Failed to retrieve missing blocks for target hash {:?}",
                        req
                    );
                    // TODO: Do we need to do anything else?
                }
            }
        }

        Poll::Pending
    }
}
