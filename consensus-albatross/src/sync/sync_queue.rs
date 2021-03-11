use std::cmp;
use std::cmp::Ordering;
use std::collections::binary_heap::PeekMut;
use std::collections::{BinaryHeap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::task::{Context, Poll};
use futures::{ready, Future, Stream, StreamExt};

use network_interface::peer::Peer;

use crate::consensus_agent::ConsensusAgent;

#[pin_project]
#[derive(Debug)]
struct OrderWrapper<TId, TOutput> {
    id: TId,
    #[pin]
    data: TOutput, // A future or a future's output
    index: usize,
    peer: usize,      // The peer the data is requested from
    num_tries: usize, // The number of tries this id has been requested
}

impl<TId: Clone, TOutput: Future> Future for OrderWrapper<TId, TOutput> {
    type Output = OrderWrapper<TId, TOutput::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let id = self.id.clone();
        let index = self.index;
        let peer = self.peer;
        let num_tries = self.num_tries;
        self.project().data.poll(cx).map(|output| OrderWrapper {
            id,
            data: output,
            index,
            peer,
            num_tries,
        })
    }
}

struct QueuedOutput<TOutput> {
    data: TOutput,
    index: usize,
}

impl<TOutput> PartialEq for QueuedOutput<TOutput> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}
impl<TOutput> Eq for QueuedOutput<TOutput> {}
impl<TOutput> PartialOrd for QueuedOutput<TOutput> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<TOutput> Ord for QueuedOutput<TOutput> {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max heap, so compare backwards here.
        other.index.cmp(&self.index)
    }
}

/// The SyncQueue will request a list of ids from a set of peers
/// and implements an ordered stream over the resulting objects.
/// The stream returns an error if an id could not be resolved.
pub struct SyncQueue<TPeer: Peer, TId, TOutput> {
    pub(crate) peers: Vec<Weak<ConsensusAgent<TPeer>>>,
    desired_pending_size: usize,
    ids_to_request: VecDeque<TId>,
    pending_futures: FuturesUnordered<OrderWrapper<TId, BoxFuture<'static, Option<TOutput>>>>,
    queued_outputs: BinaryHeap<QueuedOutput<TOutput>>,
    next_incoming_index: usize,
    next_outgoing_index: usize,
    current_peer_index: usize,
    request_fn: fn(TId, Arc<ConsensusAgent<TPeer>>) -> BoxFuture<'static, Option<TOutput>>,
}

impl<TPeer, TId, TOutput> SyncQueue<TPeer, TId, TOutput>
where
    TPeer: Peer,
    TId: Clone,
    TOutput: Send + Unpin,
{
    pub fn new(
        ids: Vec<TId>,
        peers: Vec<Weak<ConsensusAgent<TPeer>>>,
        desired_pending_size: usize,
        request_fn: fn(TId, Arc<ConsensusAgent<TPeer>>) -> BoxFuture<'static, Option<TOutput>>,
    ) -> Self {
        SyncQueue {
            peers,
            desired_pending_size,
            ids_to_request: VecDeque::from(ids),
            pending_futures: FuturesUnordered::new(),
            queued_outputs: BinaryHeap::new(),
            next_incoming_index: 0,
            next_outgoing_index: 0,
            current_peer_index: 0,
            request_fn,
        }
    }

    fn get_next_peer(&mut self, start_index: usize) -> Option<Arc<ConsensusAgent<TPeer>>> {
        while !self.peers.is_empty() {
            let index = start_index % self.peers.len();
            match Weak::upgrade(&self.peers[index]) {
                Some(peer) => {
                    return Some(peer);
                }
                None => {
                    self.peers.remove(index);
                }
            }
        }
        None
    }

    fn try_push_futures(&mut self) {
        // Determine number of new futures required to maintain desired_pending_size.
        let num_ids_to_request = cmp::min(
            self.ids_to_request.len(), // At most all of the ids
            // The number of pending futures can be higher than the desired pending size
            // (e.g., if there is an error and we re-request)
            self.desired_pending_size
                .saturating_sub(self.pending_futures.len() + self.queued_outputs.len()),
        );

        // Drain ids and produce futures.
        for _ in 0..num_ids_to_request {
            // Get next peer in line. Abort if there are no more peers.
            let peer = match self.get_next_peer(self.current_peer_index) {
                Some(peer) => peer,
                None => return,
            };

            let id = self.ids_to_request.pop_front().unwrap();
            let wrapper = OrderWrapper {
                data: (self.request_fn)(id.clone(), peer),
                id,
                index: self.next_incoming_index,
                peer: self.current_peer_index,
                num_tries: 1,
            };

            self.next_incoming_index += 1;
            self.current_peer_index = (self.current_peer_index + 1) % self.peers.len();

            self.pending_futures.push(wrapper);
        }
    }

    pub fn add_peer(&mut self, peer: Weak<ConsensusAgent<TPeer>>) {
        self.peers.push(peer);
    }

    pub fn has_peer(&self, peer: &Weak<ConsensusAgent<TPeer>>) -> bool {
        self.peers.iter().any(|o_peer| o_peer.ptr_eq(peer))
    }

    pub fn add_ids(&mut self, ids: Vec<TId>) {
        for id in ids {
            self.ids_to_request.push_back(id);
        }
    }

    /// Truncates the stored ids, retaining only the first `len` elements.
    /// The elements are counted from the *original* start of the ids vector.
    pub fn truncate_ids(&mut self, len: usize) {
        self.ids_to_request
            .truncate(len.saturating_sub(self.next_incoming_index));
    }

    pub fn num_peers(&self) -> usize {
        self.peers.len()
    }

    pub fn len(&self) -> usize {
        self.ids_to_request.len() + self.pending_futures.len() + self.queued_outputs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<TPeer, TId, TOutput> Stream for SyncQueue<TPeer, TId, TOutput>
where
    TPeer: Peer,
    TId: Clone + Unpin,
    TOutput: Send + Unpin,
{
    type Item = Result<TOutput, TId>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.try_push_futures();

        // Check to see if we've already received the next value
        let this = &mut *self;
        if let Some(next_output) = this.queued_outputs.peek_mut() {
            if next_output.index == this.next_outgoing_index {
                this.next_outgoing_index += 1;
                return Poll::Ready(Some(Ok(PeekMut::pop(next_output).data)));
            }
        }

        loop {
            match ready!(self.pending_futures.poll_next_unpin(cx)) {
                Some(result) => {
                    match result.data {
                        Some(output) => {
                            if result.index == self.next_outgoing_index {
                                self.next_outgoing_index += 1;
                                return Poll::Ready(Some(Ok(output)));
                            } else {
                                self.queued_outputs.push(QueuedOutput {
                                    data: output,
                                    index: result.index,
                                });
                            }
                        }
                        None => {
                            // If we tried all peers for this hash, return an error.
                            // TODO max number of tries
                            if result.num_tries >= self.peers.len() {
                                return Poll::Ready(Some(Err(result.id)));
                            }

                            // Re-request from different peer. Return an error if there are no more peers.
                            let next_peer = (result.peer + 1) % self.peers.len();
                            let peer = match self.get_next_peer(next_peer) {
                                Some(peer) => peer,
                                None => return Poll::Ready(Some(Err(result.id))),
                            };

                            let wrapper = OrderWrapper {
                                data: (self.request_fn)(result.id.clone(), peer),
                                id: result.id,
                                index: result.index,
                                peer: next_peer,
                                num_tries: result.num_tries + 1,
                            };

                            self.pending_futures.push(wrapper);
                        }
                    }
                }
                None => {
                    return if self.ids_to_request.is_empty() || self.peers.is_empty() {
                        Poll::Ready(None)
                    } else {
                        self.try_push_futures();
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
