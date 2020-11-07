use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{FusedStream, SelectAll};
use futures::task::{Context, Poll};
use futures::{future, ready, stream, Stream, StreamExt, TryFutureExt};
use tokio::sync::broadcast::{Receiver as BroadcastReceiver, RecvError as BroadcastRecvError};

use crate::message::Message;
use crate::peer::*;

pub enum NetworkEvent<P> {
    PeerJoined(Arc<P>),
    PeerLeft(Arc<P>),
}

impl<P> Clone for NetworkEvent<P> {
    fn clone(&self) -> Self {
        match self {
            NetworkEvent::PeerJoined(peer) => NetworkEvent::PeerJoined(Arc::clone(peer)),
            NetworkEvent::PeerLeft(peer) => NetworkEvent::PeerLeft(Arc::clone(peer)),
        }
    }
}

#[async_trait]
pub trait Network: Send + Sync + 'static {
    type PeerType: Peer + 'static;

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>>;
    fn get_peer(&self, peer_id: &<Self::PeerType as Peer>::Id) -> Option<Arc<Self::PeerType>>;

    fn subscribe_events(&self) -> BroadcastReceiver<NetworkEvent<Self::PeerType>>;

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
}

// .next() To get next item of stream.

/// A wrapper around `SelectAll` that automatically subscribes to new peers.
pub struct ReceiveFromAll<T: Message, P> {
    inner: SelectAll<Pin<Box<dyn Stream<Item = (T, Arc<P>)> + Send>>>,
    event_stream: Pin<Box<dyn FusedStream<Item = Result<NetworkEvent<P>, BroadcastRecvError>> + Send>>,
}

impl<T: Message, P: Peer + 'static> ReceiveFromAll<T, P> {
    pub fn new<N: Network<PeerType = P> + ?Sized>(network: &N) -> Self {
        ReceiveFromAll {
            inner: stream::select_all(network.get_peers().iter().map(|peer| {
                let peer_inner = Arc::clone(&peer);
                peer.receive::<T>().map(move |item| (item, Arc::clone(&peer_inner))).boxed()
            })),
            event_stream: Box::pin(network.subscribe_events().into_stream().fuse()),
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
                Poll::Ready(Some(Err(BroadcastRecvError::Lagged(_)))) => {}
                Poll::Ready(None) | Poll::Ready(Some(Err(BroadcastRecvError::Closed))) => {
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
