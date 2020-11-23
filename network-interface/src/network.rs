use std::{
    pin::Pin,
    sync::Arc,
    collections::HashMap,
};

use async_trait::async_trait;
use futures::{
    task::{Context, Poll},
    stream::{FusedStream, SelectAll},
    future, ready, Stream, StreamExt, TryFutureExt, stream
};
use tokio::sync::broadcast;
use parking_lot::RwLock;

use beserial::{Serialize, Deserialize};

use crate::message::Message;
use crate::peer::*;

pub enum NetworkEvent<P> {
    PeerJoined(Arc<P>),
    PeerLeft(Arc<P>),
}

pub trait Topic {
    type Item: Serialize + Deserialize + Send;

    fn topic(&self) -> String;
}

impl<P: Peer> std::fmt::Debug for NetworkEvent<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let (event_name, peer) = match self {
            NetworkEvent::PeerJoined(peer) => ("PeerJoined", peer),
            NetworkEvent::PeerLeft(peer) => ("PeerLeft", peer),
        };

        f.debug_struct(event_name)
            .field("peer_id", &peer.id())
            .finish()
    }
}

impl<P> Clone for NetworkEvent<P> {
    fn clone(&self) -> Self {
        match self {
            NetworkEvent::PeerJoined(peer) => NetworkEvent::PeerJoined(Arc::clone(peer)),
            NetworkEvent::PeerLeft(peer) => NetworkEvent::PeerLeft(Arc::clone(peer)),
        }
    }
}


struct Inner<P>
    where P: Peer + 'static,
{
    peers: HashMap<P::Id, Arc<P>>,
    tx: broadcast::Sender<NetworkEvent<P>>,
}

impl<P> Inner<P>
    where P: Peer + 'static,
{
    fn notify(&self, event: NetworkEvent<P>) {
        // According to documentation this only fails if all receivers dropped. But that's okay for us.
        self.tx.send(event).ok();
    }
}

pub struct ObservablePeerMap<P>
    where P: Peer + 'static,
{
    inner: Arc<RwLock<Inner<P>>>,
}

impl<P> Clone for ObservablePeerMap<P>
    where P: Peer + std::fmt::Debug,
{
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<P> std::fmt::Debug for ObservablePeerMap<P>
    where P: Peer + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        self.inner.read().peers.fmt(f)
    }
}

impl<P> Default for ObservablePeerMap<P>
    where P: Peer,
{
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<P> ObservablePeerMap<P>
    where P: Peer + 'static,
{
    pub fn new(peers: HashMap<P::Id, Arc<P>>) -> Self {
        let (tx, _rx) = broadcast::channel(64);

        Self {
            inner: Arc::new(RwLock::new(Inner {
                peers,
                tx,
            }))
        }
    }

    pub fn insert(&self, peer: Arc<P>) {
        let mut inner = self.inner.write();

        inner.peers.insert(peer.id(), Arc::clone(&peer));
        inner.notify(NetworkEvent::PeerJoined(peer));
    }

    pub fn remove(&self, peer_id: &P::Id) -> Option<Arc<P>> {
        let mut inner = self.inner.write();

        if let Some(peer) = inner.peers.remove(peer_id) {
            inner.notify(NetworkEvent::PeerLeft(Arc::clone(&peer)));
            Some(peer)
        }
        else {
            None
        }
    }

    pub fn get_peer(&self, peer_id: &P::Id) -> Option<Arc<P>> {
        self.inner
            .read()
            .peers
            .get(peer_id)
            .map(|peer| Arc::clone(peer))
    }

    pub fn get_peers(&self) -> Vec<Arc<P>> {
        self.inner
            .read()
            .peers
            .values()
            .map(|peer| Arc::clone(peer))
            .collect()
    }

    pub fn subscribe(&self) -> (Vec<Arc<P>>, broadcast::Receiver<NetworkEvent<P>>) {
        let inner = self.inner.write();

        let peers = inner.peers
            .values()
            .map(|peer| Arc::clone(peer))
            .collect();

        let rx = inner.tx.subscribe();

        (peers, rx)
    }
}


#[async_trait]
pub trait Network: Send + Sync + 'static {
    type PeerType: Peer + 'static;
    type Error: std::error::Error;

    fn get_peer_updates(&self) -> (Vec<Arc<Self::PeerType>>, broadcast::Receiver<NetworkEvent<Self::PeerType>>);

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>>;
    fn get_peer(&self, peer_id: <Self::PeerType as Peer>::Id) -> Option<Arc<Self::PeerType>>;

    fn subscribe_events(&self) -> broadcast::Receiver<NetworkEvent<Self::PeerType>>;

    async fn broadcast<T: Message>(&self, msg: &T) {
        future::join_all(self.get_peers().iter().map(|peer| {
            // TODO: Close reason
            peer.send_or_close(msg, |_| CloseReason::Other).unwrap_or_else(|_| ())
        }))
        .await;
    }

    /// Should panic if there is already a non-closed sink registered for a message type.
    fn receive_from_all<T: Message>(&self) -> ReceiveFromAll<T, Self::PeerType> {
        ReceiveFromAll::new(self)
    }

    async fn subscribe<T>(topic: &T) -> Box<dyn Stream<Item = (T::Item, Self::PeerType)> + Send>
        where
            T: Topic + Sync;

    async fn publish<T: Topic>(topic: &T, item: T::Item)
        where
            T: Topic + Sync;

    async fn dht_get<K, V>(&self, k: &K) -> Result<V, Self::Error>
        where
            K: AsRef<[u8]> + Send + Sync,
            V: Deserialize + Send + Sync;

    async fn dht_put<K, V>(&self, k: &K, v: &V) -> Result<(), Self::Error>
        where
            K: AsRef<[u8]> + Send + Sync,
            V: Serialize + Send + Sync;

    // TODO: dial
}

// .next() To get next item of stream.

/// A wrapper around `SelectAll` that automatically subscribes to new peers.
pub struct ReceiveFromAll<T: Message, P> {
    inner: SelectAll<Pin<Box<dyn Stream<Item = (T, Arc<P>)> + Send>>>,
    event_stream: Pin<Box<dyn FusedStream<Item = Result<NetworkEvent<P>, broadcast::RecvError>> + Send>>,
}

impl<T: Message, P: Peer + 'static> ReceiveFromAll<T, P> {
    pub fn new<N: Network<PeerType = P> + ?Sized>(network: &N) -> Self {
        let (peers, updates) = network.get_peer_updates();

        ReceiveFromAll {
            inner: stream::select_all(peers.into_iter().map(|peer| {
                let peer_inner = Arc::clone(&peer);
                peer.receive::<T>().map(move |item| (item, Arc::clone(&peer_inner))).boxed()
            })),
            event_stream: Box::pin(updates.into_stream().fuse()),
        }
    }
}

impl<T: Message, P: Peer + 'static> Stream for ReceiveFromAll<T, P> {
    type Item = (T, Arc<P>);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.event_stream.poll_next_unpin(cx) {
                Poll::Pending => break,
                Poll::Ready(Some(Ok(NetworkEvent::PeerJoined(peer)))) => {
                    // We have a new peer to receive from.
                    let peer_inner = Arc::clone(&peer);
                    self.inner.push(peer.receive::<T>().map(move |item| (item, Arc::clone(&peer_inner))).boxed())
                }
                #[allow(unreachable_patterns)]
                Poll::Ready(Some(Ok(_))) => {} // Ignore others.
                // The receiver lagged too far behind.
                // Attempting to receive again will return the oldest message still retained by the channel.
                // So, that's what we do.
                Poll::Ready(Some(Err(broadcast::RecvError::Lagged(_)))) => {}
                Poll::Ready(None) | Poll::Ready(Some(Err(broadcast::RecvError::Closed))) => {
                    // There are no more active senders implying no further messages will ever be sent.
                    return Poll::Ready(None); // Discard this stream entirely.
                }
            }
        }
        match ready!(self.inner.poll_next_unpin(cx)) {
            // `SelectAll` is built upon a `FuturesUnordered`, which returns `None` once the list of
            // futures is all worked through. However, it allows to add new futures and then
            // may resume to return values.
            // Thus, it is fine for us to return `Pending` once all streams in the select all are
            // gone as we know `SelectAll` can actually recover from the `None` case once we
            // push new streams into it.
            None => Poll::Pending,
            other => Poll::Ready(other),
        }
    }
}

impl<T: Message, P: Peer + 'static> FusedStream for ReceiveFromAll<T, P> {
    fn is_terminated(&self) -> bool {
        self.event_stream.is_terminated()
    }
}
