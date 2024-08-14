use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, FutureExt, Stream, StreamExt};
use nimiq_network_interface::{network::Network, request::RequestError};
use nimiq_primitives::key_nibbles::KeyNibbles;
use parking_lot::RwLock;

use super::{RequestChunk, ResponseChunk};
use crate::sync::{peer_list::PeerList, sync_queue::SyncQueue};

/// Peer Tracking & Chunk Request Component.
/// This component returns only the responses that respect the size limit specified on
/// the respective request.
///
/// This component has:
///
/// - The sync queue which manages the requests and responses.
/// - The peers list.
/// - The network stream of events used to remove the peers that have left.  
///
/// The public interface allows to request chunks, which are not immediately returned.
/// The chunks instead are returned by polling the component.
pub struct ChunkRequestComponent<N: Network> {
    sync_queue:
        SyncQueue<N, RequestChunk, (ResponseChunk, RequestChunk, N::PeerId), RequestError, ()>,
    // These peers will be shared across the block request component and this component.
    peers: Arc<RwLock<PeerList<N>>>,
}

impl<N: Network> ChunkRequestComponent<N> {
    const NUM_PENDING_CHUNKS: usize = 1;

    pub fn new(network: Arc<N>, peers: Arc<RwLock<PeerList<N>>>) -> Self {
        let sync_queue = SyncQueue::new(
            network,
            vec![],
            Arc::clone(&peers),
            Self::NUM_PENDING_CHUNKS,
            |request_chunk: RequestChunk, network, peer_id| {
                async move {
                    Self::request_missing_chunks_from_peer(network, peer_id, request_chunk.clone())
                        .await
                        .map(|res| (res, request_chunk, peer_id))
                }
                .boxed()
            },
        );

        ChunkRequestComponent { sync_queue, peers }
    }

    pub fn remove_peer(&mut self, peer_id: &N::PeerId) {
        self.peers.write().remove_peer(peer_id);
    }

    pub fn has_pending_requests(&self) -> bool {
        !self.sync_queue.is_empty()
    }

    pub fn request_chunk(&mut self, request: RequestChunk) {
        self.sync_queue.add_ids(vec![(request, None)]);
    }

    async fn request_missing_chunks_from_peer(
        network: Arc<N>,
        peer_id: N::PeerId,
        request: RequestChunk,
    ) -> Result<ResponseChunk, RequestError> {
        network.request::<RequestChunk>(request, peer_id).await
    }

    /// Returns a future that resolves when the peer list of the chunk request
    /// component becomes nonempty.
    ///
    /// Returns `None` is the chunk request component already has peers.
    pub fn wait_for_peers(&self) -> Option<BoxFuture<'static, ()>> {
        self.peers.read().wait_for_peers()
    }
}

impl<N: Network> Stream for ChunkRequestComponent<N> {
    type Item = (ResponseChunk, KeyNibbles, N::PeerId);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll self.sync_queue, return results.
        while let Poll::Ready(Some(result)) = self.sync_queue.poll_next_unpin(cx) {
            match result {
                Ok((response, request, peer_id)) => {
                    // Verifies the response chunk size.
                    if let ResponseChunk::Chunk(ref chunk) = response {
                        if chunk.chunk.items.len() > request.limit as usize {
                            debug!(
                                    "Peer[{}] Chunk size exceeded the request limit. Req: {:?} Chunk size: {}",
                                    peer_id,
                                    request,
                                    chunk.chunk.items.len()
                                );
                            // TODO: Ban peer
                            continue;
                        }
                    }

                    return Poll::Ready(Some((response, request.start_key, peer_id)));
                }
                Err(req) => {
                    debug!(
                        "Failed to retrieve missing chunks for target hash {:?}",
                        req
                    );
                    // TODO: Do we need to do anything else?
                }
            }
        }

        Poll::Pending
    }
}
