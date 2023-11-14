use std::{
    cmp,
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
    fmt::Debug,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{
    future, future::BoxFuture, ready, stream::FuturesUnordered, FutureExt, Stream, StreamExt,
};
use nimiq_macros::store_waker;
use nimiq_network_interface::network::{Network, PubsubId};
use parking_lot::RwLock;
use pin_project::pin_project;

use super::peer_list::PeerList;
use crate::sync::peer_list::PeerListIndex;

#[pin_project]
#[derive(Debug)]
struct OrderWrapper<TId, TOutput> {
    id: TId,
    #[pin]
    data: TOutput, // A future or a future's output
    index: usize,
    peer: PeerListIndex, // The peer the data is requested from
    num_tries: usize,    // The number of tries this id has been requested
}

impl<TId: Clone, TOutput: Future> Future for OrderWrapper<TId, TOutput> {
    type Output = OrderWrapper<TId, TOutput::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let id = self.id.clone();
        let index = self.index;
        let peer = self.peer.clone();
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

impl<TId, TOutput> PartialEq for OrderWrapper<TId, TOutput> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}
impl<TId, TOutput> Eq for OrderWrapper<TId, TOutput> {}
impl<TId, TOutput> PartialOrd for OrderWrapper<TId, TOutput> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<TId, TOutput> Ord for OrderWrapper<TId, TOutput> {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max heap, so compare backwards here.
        other.index.cmp(&self.index)
    }
}

type RequestFn<TId, TNetwork, TOutput> =
    fn(TId, Arc<TNetwork>, <TNetwork as Network>::PeerId) -> BoxFuture<'static, Option<TOutput>>;
type VerifyFn<TId, TOutput, TVerifyState> = fn(&TId, &TOutput, &mut TVerifyState) -> bool;

/// The SyncQueue will request a list of ids from a set of peers
/// and implements an ordered stream over the resulting objects.
/// The stream returns an error if an id could not be resolved.
pub struct SyncQueue<TNetwork: Network, TId, TOutput: 'static, TVerifyState: 'static> {
    pub(crate) peers: Arc<RwLock<PeerList<TNetwork>>>,
    network: Arc<TNetwork>,
    desired_pending_size: usize,
    ids_to_request: VecDeque<(TId, Option<TNetwork::PubsubId>)>,
    pending_futures: FuturesUnordered<OrderWrapper<TId, BoxFuture<'static, Option<TOutput>>>>,
    queued_outputs: BinaryHeap<OrderWrapper<TId, Option<TOutput>>>,
    next_incoming_index: usize,
    next_outgoing_index: usize,
    current_peer_index: PeerListIndex,
    request_fn: RequestFn<TId, TNetwork, TOutput>,
    verify_fn: VerifyFn<TId, TOutput, TVerifyState>,
    verify_state: TVerifyState,
    waker: Option<Waker>,
}

impl<TNetwork, TId, TOutput> SyncQueue<TNetwork, TId, TOutput, ()>
where
    TId: Clone + Debug,
    TOutput: Send + Unpin + 'static,
    TNetwork: Network,
{
    pub fn new(
        network: Arc<TNetwork>,
        ids: Vec<(TId, Option<TNetwork::PubsubId>)>,
        peers: Arc<RwLock<PeerList<TNetwork>>>,
        desired_pending_size: usize,
        request_fn: RequestFn<TId, TNetwork, TOutput>,
    ) -> Self {
        Self::with_verification(
            network,
            ids,
            peers,
            desired_pending_size,
            request_fn,
            |_, _, _| true,
            (),
        )
    }
}

impl<TNetwork, TId, TOutput, TVerifyState> SyncQueue<TNetwork, TId, TOutput, TVerifyState>
where
    TId: Clone + Debug,
    TOutput: Send + Unpin + 'static,
    TNetwork: Network,
{
    pub fn with_verification(
        network: Arc<TNetwork>,
        ids: Vec<(TId, Option<TNetwork::PubsubId>)>,
        peers: Arc<RwLock<PeerList<TNetwork>>>,
        desired_pending_size: usize,
        request_fn: RequestFn<TId, TNetwork, TOutput>,
        verify_fn: VerifyFn<TId, TOutput, TVerifyState>,
        initial_verify_state: TVerifyState,
    ) -> Self {
        log::trace!(
            "Creating SyncQueue for {} with {} ids and {} peers",
            std::any::type_name::<TOutput>(),
            ids.len(),
            peers.read().len(),
        );

        SyncQueue {
            network,
            peers,
            desired_pending_size,
            ids_to_request: VecDeque::from(ids),
            pending_futures: FuturesUnordered::new(),
            queued_outputs: BinaryHeap::new(),
            next_incoming_index: 0,
            next_outgoing_index: 0,
            current_peer_index: PeerListIndex::default(),
            request_fn,
            verify_fn,
            verify_state: initial_verify_state,
            waker: None,
        }
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
            let id = self.ids_to_request.pop_front().unwrap().0;
            let pubsub_id = self.ids_to_request.pop_front().unwrap().1;

            // If we know the peer that sent us this block, we ask them first.
            let peer = match pubsub_id {
                Some(pubsub_id) => {
                    let peer_id = pubsub_id.propagation_source();

                    self.peers
                        .read()
                        .index_of(&peer_id)
                        .map(|peer_index| (peer_id, peer_index))
                }
                None => self
                    .peers
                    .read()
                    .increment_and_get(&mut self.current_peer_index)
                    .map(|peer_id| (peer_id, self.current_peer_index.clone())),
            };

            let wrapper = match peer {
                Some((peer_id, peer_index)) => {
                    log::trace!(
                        %peer_id,
                        current_peer_index = %peer_index,
                        "Requesting {:?} @ {}",
                        id,
                        self.next_incoming_index,
                    );

                    OrderWrapper {
                        data: (self.request_fn)(id.clone(), Arc::clone(&self.network), peer_id),
                        id,
                        index: self.next_incoming_index,
                        peer: peer_index,
                        num_tries: 1,
                    }
                }
                None => OrderWrapper {
                    data: future::ready(None).boxed(),
                    id,
                    index: self.next_incoming_index,
                    peer: PeerListIndex::default(),
                    num_tries: 1,
                },
            };

            self.next_incoming_index += 1;

            self.pending_futures.push(wrapper);
        }

        if num_ids_to_request > 0 {
            log::trace!(
                "Requesting {} ids (ids_to_request={}, remaining_until_limit={}, pending_futures={}, queued_outputs={}, num_peers={})",
                num_ids_to_request,
                self.ids_to_request.len(),
                self.desired_pending_size
                    .saturating_sub(self.pending_futures.len() + self.queued_outputs.len()),
                self.pending_futures.len(),
                self.queued_outputs.len(),
                self.peers.read().len(),
            );

            if let Some(waker) = self.waker.take() {
                waker.wake();
            }
        }
    }

    fn retry_request(&mut self, mut request: OrderWrapper<TId, Option<TOutput>>) -> bool {
        // If we tried all peers for this hash, return an error.
        // TODO max number of tries
        if request.num_tries >= self.peers.read().len() {
            return false;
        }

        // Re-request from different peer. Return an error if there are no more peers.
        let peer = match self.peers.read().increment_and_get(&mut request.peer) {
            Some(peer) => peer,
            None => return false,
        };

        log::debug!(
            peer_id = %peer,
            current_peer_index = %self.current_peer_index,
            "Re-requesting {:?} @ {}",
            request.id,
            request.index,
        );

        let wrapper = OrderWrapper {
            data: (self.request_fn)(request.id.clone(), Arc::clone(&self.network), peer),
            id: request.id,
            index: request.index,
            peer: request.peer,
            num_tries: request.num_tries + 1,
        };

        self.pending_futures.push(wrapper);

        true
    }

    pub fn add_peer(&mut self, peer_id: TNetwork::PeerId) -> bool {
        self.peers.write().add_peer(peer_id)
    }

    pub fn remove_peer(&mut self, peer_id: &TNetwork::PeerId) {
        self.peers.write().remove_peer(peer_id);
    }

    pub fn add_ids(&mut self, ids: Vec<(TId, Option<TNetwork::PubsubId>)>) {
        for id in ids {
            self.ids_to_request.push_back(id);
        }

        // Adding new ids needs to wake the task that is polling the SyncQueue.
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// Truncates the stored ids, retaining only the first `len` elements.
    /// The elements are counted from the *original* start of the ids vector.
    pub fn truncate_ids(&mut self, len: usize) {
        self.ids_to_request
            .truncate(len.saturating_sub(self.next_incoming_index));
    }

    pub fn num_peers(&self) -> usize {
        self.peers.read().len()
    }

    pub fn len(&self) -> usize {
        self.ids_to_request.len() + self.pending_futures.len() + self.queued_outputs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn set_verify_state(&mut self, verify_state: TVerifyState) {
        self.verify_state = verify_state;
    }
}

impl<TNetwork, TId, TOutput, TVerifyState> Stream
    for SyncQueue<TNetwork, TId, TOutput, TVerifyState>
where
    TNetwork: Network,
    TId: Clone + Unpin + Debug,
    TOutput: Send + Unpin,
    TVerifyState: Unpin + 'static,
{
    type Item = Result<TOutput, TId>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        store_waker!(self, waker, cx);

        // Try to request more objects.
        self.try_push_futures();

        // Check to see if we've already received the next value.
        if let Some(next_output) = self.queued_outputs.peek() {
            if next_output.index == self.next_outgoing_index {
                let mut request = self.queued_outputs.pop().unwrap();
                let output = request.data.take().unwrap();
                if (self.verify_fn)(&request.id, &output, &mut self.verify_state) {
                    self.next_outgoing_index += 1;
                    return Poll::Ready(Some(Ok(output)));
                } else {
                    debug!(peer_id = %request.peer, id = ?request.id, "Verification failed");
                    let id = request.id.clone();
                    if !self.retry_request(request) {
                        return Poll::Ready(Some(Err(id)));
                    }
                }
            }
        }

        loop {
            match ready!(self.pending_futures.poll_next_unpin(cx)) {
                Some(result) => {
                    if result.data.is_some() {
                        if result.index == self.next_outgoing_index {
                            let output = result.data.as_ref().unwrap();
                            if (self.verify_fn)(&result.id, output, &mut self.verify_state) {
                                self.next_outgoing_index += 1;
                                return Poll::Ready(Some(Ok(result.data.unwrap())));
                            } else {
                                debug!(peer_id = %result.peer, id = ?result.id, "Verification failed");
                            }
                        } else {
                            self.queued_outputs.push(result);
                            continue;
                        }
                    }

                    // The request or verification failed.
                    let id = result.id.clone();
                    if !self.retry_request(result) {
                        return Poll::Ready(Some(Err(id)));
                    }
                }
                None => {
                    return if self.ids_to_request.is_empty() || self.peers.read().is_empty() {
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

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        task::{Context, Poll},
    };

    use futures::{future, task::noop_waker_ref, FutureExt, StreamExt};
    use nimiq_network_mock::MockHub;

    use crate::sync::sync_queue::SyncQueue;

    #[test]
    fn it_can_handle_no_peers() {
        let mut hub = MockHub::new();
        let network = Arc::new(hub.new_network());

        let mut queue: SyncQueue<_, _, i32, _> = SyncQueue::new(
            network,
            vec![(1, None), (2, None), (3, None), (4, None)],
            Default::default(),
            1,
            |_, _, _| future::ready(None).boxed(),
        );

        match queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref())) {
            Poll::Ready(Some(Err(id))) => assert_eq!(id, 1),
            _ => panic!("Expected error"),
        };
    }
}
