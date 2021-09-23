use std::{pin::Pin, sync::Arc};

use async_trait::async_trait;
use futures::{
    future, ready, stream,
    stream::{BoxStream, FusedStream, SelectAll},
    task::{Context, Poll},
    Stream, StreamExt, TryFutureExt,
};
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

use beserial::{Deserialize, Serialize};

use crate::message::Message;
use crate::peer::*;

pub enum NetworkEvent<P> {
    PeerJoined(Arc<P>),
    PeerLeft(Arc<P>),
}

pub trait Topic {
    type Item: Serialize + Deserialize + Send + Sync + std::fmt::Debug + 'static;

    const BUFFER_SIZE: usize;
    const NAME: &'static str;
    const VALIDATE: bool;
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

// It seems we can't use type aliases on enums yet:
// https://rust-lang.github.io/rfcs/2338-type-alias-enum-variants.html
#[derive(Clone, Debug)]
pub enum MsgAcceptance {
    Accept,
    Reject,
    Ignore,
}

pub trait PubsubId<PeerId>: Send + Sync {
    fn propagation_source(&self) -> PeerId;
}

#[async_trait]
pub trait Network: Send + Sync + 'static {
    type PeerType: Peer + 'static;
    type AddressType: std::fmt::Display + std::fmt::Debug;
    type Error: std::error::Error;
    type PubsubId: PubsubId<<Self::PeerType as Peer>::Id>;

    fn get_peer_updates(
        &self,
    ) -> (
        Vec<Arc<Self::PeerType>>,
        BroadcastStream<NetworkEvent<Self::PeerType>>,
    );

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>>;
    fn get_peer(&self, peer_id: <Self::PeerType as Peer>::Id) -> Option<Arc<Self::PeerType>>;

    fn subscribe_events(&self) -> BroadcastStream<NetworkEvent<Self::PeerType>>;

    async fn broadcast<T: Message>(&self, msg: &T) {
        future::join_all(self.get_peers().iter().map(|peer| {
            // TODO: Close reason
            peer.send_or_close(msg, |_| CloseReason::Other)
                .unwrap_or_else(|_| ())
        }))
        .await;
    }

    /// Should panic if there is already a non-closed sink registered for a message type.
    fn receive_from_all<'a, T: Message>(&self) -> BoxStream<'a, (T, Arc<Self::PeerType>)> {
        ReceiveFromAll::new(self).boxed()
    }

    async fn subscribe<'a, T>(
        &self,
    ) -> Result<BoxStream<'a, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync;

    async fn unsubscribe<'a, T>(&self) -> Result<(), Self::Error>
    where
        T: Topic + Sync;

    async fn publish<T>(&self, item: T::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync;

    async fn validate_message(
        &self,
        id: Self::PubsubId,
        acceptance: MsgAcceptance,
    ) -> Result<bool, Self::Error>;

    async fn dht_get<K, V>(&self, k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync;

    async fn dht_put<K, V>(&self, k: &K, v: &V) -> Result<(), Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Serialize + Send + Sync;

    async fn dial_peer(&self, peer_id: <Self::PeerType as Peer>::Id) -> Result<(), Self::Error>;

    async fn dial_address(&self, address: Self::AddressType) -> Result<(), Self::Error>;

    fn get_local_peer_id(&self) -> <Self::PeerType as Peer>::Id;
}

// .next() To get next item of stream.

/// A wrapper around `SelectAll` that automatically subscribes to new peers.
pub struct ReceiveFromAll<T: Message, P> {
    inner: SelectAll<Pin<Box<dyn Stream<Item = (T, Arc<P>)> + Send>>>,
    event_stream:
        Pin<Box<dyn FusedStream<Item = Result<NetworkEvent<P>, BroadcastStreamRecvError>> + Send>>,
}

impl<T: Message, P: Peer + 'static> ReceiveFromAll<T, P> {
    pub fn new<N: Network<PeerType = P> + ?Sized>(network: &N) -> Self {
        let (peers, updates) = network.get_peer_updates();
        //log::trace!("peers = {:?}", peers.iter().map(|peer| peer.id()).collect::<Vec<_>>());

        ReceiveFromAll {
            inner: stream::select_all(peers.into_iter().map(|peer| {
                let peer_inner = Arc::clone(&peer);
                peer.receive::<T>()
                    .map(move |item| (item, Arc::clone(&peer_inner)))
                    .boxed()
            })),
            event_stream: Box::pin(updates.fuse()),
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
                    log::trace!("peers joined {:?}", peer.id());
                    // We have a new peer to receive from.
                    let peer_inner = Arc::clone(&peer);
                    self.inner.push(
                        peer.receive::<T>()
                            .map(move |item| (item, Arc::clone(&peer_inner)))
                            .boxed(),
                    )
                }
                #[allow(unreachable_patterns)]
                Poll::Ready(Some(Ok(_))) => {} // Ignore others.
                // The receiver lagged too far behind.
                // Attempting to receive again will return the oldest message still retained by the channel.
                // So, that's what we do.
                Poll::Ready(Some(Err(BroadcastStreamRecvError::Lagged(_)))) => {}
                Poll::Ready(None) => {
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
