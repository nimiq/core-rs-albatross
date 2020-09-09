use crate::consensus_agent::ConsensusAgent;

use futures::stream::FuturesUnordered;
use futures::task::{Context, Poll};
use futures::{ready, Future, Stream, StreamExt};
use hash::Blake2bHash;
use network_interface::peer::Peer;

use std::cmp;
use std::cmp::Ordering;
use std::collections::binary_heap::PeekMut;
use std::collections::BinaryHeap;
use std::pin::Pin;
use std::sync::Arc;

#[pin_project]
#[derive(Debug)]
struct OrderWrapper<T> {
    #[pin]
    data: T, // A future or a future's output
    index: usize,
    peer: usize,      // The peer the data is requested from
    num_tries: usize, // The number of tries this hash has been requested
    hash: Blake2bHash,
}

impl<T> PartialEq for OrderWrapper<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for OrderWrapper<T> {}

impl<T> PartialOrd for OrderWrapper<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for OrderWrapper<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max heap, so compare backwards here.
        other.index.cmp(&self.index)
    }
}

impl<T> Future for OrderWrapper<T>
where
    T: Future,
{
    type Output = OrderWrapper<T::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let index = self.index;
        let peer = self.peer;
        let num_tries = self.num_tries;
        let hash = self.hash.clone();
        self.project().data.poll(cx).map(|output| OrderWrapper {
            data: output,
            index,
            peer,
            num_tries,
            hash,
        })
    }
}

/// The SyncQueue will request a set of hashes from a set of peers
/// and implements an ordered stream over the resulting blocks.
/// The stream returns an error if a hash could not be resolved.
pub struct SyncQueue<P: Peer, T, E, F> {
    peers: Vec<Arc<ConsensusAgent<P>>>,
    desired_pending_size: usize,
    hashes_to_request: Vec<Blake2bHash>,
    pending_futures:
        FuturesUnordered<OrderWrapper<Pin<Box<dyn Future<Output = Result<T, E>> + Send>>>>,
    queued_outputs: BinaryHeap<OrderWrapper<T>>,
    next_incoming_index: usize,
    next_outgoing_index: usize,
    current_peer_index: usize,
    request_fn: F,
}

impl<
        P: Peer,
        T: Send + Unpin,
        E: Send,
        F: Fn(
                Arc<ConsensusAgent<P>>,
                Blake2bHash,
            ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>
            + Unpin,
    > SyncQueue<P, T, E, F>
{
    pub fn new(
        hashes: Vec<Blake2bHash>,
        peers: Vec<Arc<ConsensusAgent<P>>>,
        desired_pending_size: usize,
        request_fn: F,
    ) -> Self {
        SyncQueue {
            peers,
            desired_pending_size,
            hashes_to_request: hashes,
            pending_futures: FuturesUnordered::new(),
            queued_outputs: BinaryHeap::new(),
            next_outgoing_index: 0,
            next_incoming_index: 0,
            current_peer_index: 0,
            request_fn,
        }
    }

    pub fn len(&self) -> usize {
        self.hashes_to_request.len() + self.pending_futures.len() + self.queued_outputs.len()
    }

    fn try_push_futures(&mut self) {
        // Determine required number of futures for keeping backpressure
        let new_hashes_to_request = cmp::min(
            self.hashes_to_request.len(), // At most all of the hashes
            // The number of pending futures can be higher than the desired pending size
            // (e.g., if there is an error and we re-request)
            self.desired_pending_size
                .saturating_sub(self.pending_futures.len()),
        );

        // Drain hashes and produce futures
        for hash in self.hashes_to_request.drain(0..new_hashes_to_request) {
            // Request hash from next peer in line.
            let peer = &self.peers[self.current_peer_index];

            let wrapped = OrderWrapper {
                data: (self.request_fn)(Arc::clone(&peer), hash.clone()),
                index: self.next_incoming_index,
                peer: self.current_peer_index,
                num_tries: 1,
                hash,
            };

            self.next_incoming_index += 1;
            self.current_peer_index = (self.current_peer_index + 1) % self.peers.len();

            self.pending_futures.push(wrapped);
        }
    }
}

impl<
        P: Peer,
        T: Send + Unpin,
        E: Send,
        F: Fn(
                Arc<ConsensusAgent<P>>,
                Blake2bHash,
            ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>
            + Unpin,
    > Stream for SyncQueue<P, T, E, F>
{
    type Item = Result<T, Blake2bHash>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;

        this.try_push_futures();

        // Check to see if we've already received the next value
        if let Some(next_output) = this.queued_outputs.peek_mut() {
            if next_output.index == this.next_outgoing_index {
                this.next_outgoing_index += 1;
                return Poll::Ready(Some(Ok(PeekMut::pop(next_output).data)));
            }
        }

        loop {
            match ready!(this.pending_futures.poll_next_unpin(cx)) {
                Some(output) => {
                    match output.data {
                        Ok(block) => {
                            if output.index == this.next_outgoing_index {
                                this.next_outgoing_index += 1;
                                return Poll::Ready(Some(Ok(block)));
                            } else {
                                // An order wrapper with dummy information for the parts not required
                                this.queued_outputs.push(OrderWrapper {
                                    data: block,
                                    index: output.index,
                                    // The following fields are not required here
                                    peer: 0,
                                    num_tries: 0,
                                    hash: Default::default(),
                                })
                            }
                        }
                        Err(_) => {
                            // If we tried all peers for this hash, return an error
                            if output.num_tries == this.peers.len() {
                                return Poll::Ready(Some(Err(output.hash)));
                            }

                            // Re-request from different peer
                            let next_peer = (output.peer + 1) % this.peers.len();
                            let peer = &this.peers[next_peer];

                            let wrapped = OrderWrapper {
                                data: (this.request_fn)(Arc::clone(&peer), output.hash.clone()),
                                index: output.index,
                                peer: next_peer,
                                num_tries: output.num_tries + 1,
                                hash: output.hash,
                            };

                            this.pending_futures.push(wrapped);
                        }
                    }
                }
                None => {
                    return if this.hashes_to_request.is_empty() {
                        Poll::Ready(None)
                    } else {
                        this.try_push_futures();
                        Poll::Pending
                    }
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}
