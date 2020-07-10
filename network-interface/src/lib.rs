use async_trait::async_trait;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use futures::stream::{FusedStream, SelectAll};
use futures::task::{Context, Poll};
use futures::{
    channel::oneshot::{channel, Sender},
    future, stream, Stream, StreamExt, TryFutureExt,
};
use nimiq_network_primitives::address::PeerId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::{
    sync::broadcast::{Receiver as BroadcastReceiver, RecvError as BroadcastRecvError},
    task::spawn,
    time::timeout,
};

pub trait Message: Serialize + Deserialize + Send + Sync {
    const TYPE_ID: u64;

    // Does CRC stuff and is called by network
    fn serialize_message<W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        // TODO: Does CRC stuff.
        Self::TYPE_ID.serialize(writer)?; // TODO: uvar
        self.serialize(writer)
    }

    fn deserialize_message<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        // TODO: Does CRC stuff.
        let msg_type: u64 = Deserialize::deserialize(reader)?; // TODO: uvar
        assert_eq!(msg_type, Self::TYPE_ID);
        Deserialize::deserialize(reader)
    }
}

pub trait RequestMessage: Message {
    fn set_request_identifier(&mut self, request_identifier: u32);
}

pub trait ResponseMessage: Message {
    fn get_request_identifier(&self) -> u32;
}

pub enum CloseReason {
    Other,
}

pub enum SendError {
    Serialization(SerializingError),
    AlreadyClosed,
}

#[async_trait]
pub trait Peer: Send + Sync {
    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError>;
    async fn send_or_close<T: Message, F: FnOnce(&SendError) -> CloseReason + Send>(
        &self,
        msg: &T,
        f: F,
    ) -> Result<(), SendError> {
        if let Err(e) = self.send(msg).await {
            self.close(f(&e)).await;
            Err(e)
        } else {
            Ok(())
        }
    }
    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>>;
    async fn close(&self, ty: CloseReason);
}

pub enum NetworkEvent<P> {
    PeerJoined(Arc<P>),
    //PeerLeft(Arc<P>),
}

impl<P> Clone for NetworkEvent<P> {
    fn clone(&self) -> Self {
        match self {
            NetworkEvent::PeerJoined(peer) => NetworkEvent::PeerJoined(Arc::clone(peer)),
        }
    }
}

#[async_trait]
pub trait Network {
    type PeerType: Peer + 'static;

    fn get_peers(&self) -> &[Arc<Self::PeerType>];
    fn get_peer(&self, peer_id: PeerId) -> &Arc<Self::PeerType>;

    fn subscribe_events(&self) -> BroadcastReceiver<NetworkEvent<Self::PeerType>>;

    async fn broadcast<T: Message>(&self, msg: &T) {
        future::join_all(self.get_peers().iter().map(|peer| {
            // TODO: Close reason
            peer.send_or_close(msg, |_| CloseReason::Other)
                .unwrap_or_else(|_| ())
        }))
        .await;
    }

    fn receive_from_all<T: Message>(&self) -> ReceiveFromAll<T, Self::PeerType> {
        ReceiveFromAll::new(self)
    }
}

// .next() To get next item of stream.

/// A wrapper around `SelectAll` that automatically subscribes to new peers.
pub struct ReceiveFromAll<T: Message, P> {
    inner: SelectAll<Pin<Box<dyn Stream<Item = T> + Send>>>,
    event_stream:
        Pin<Box<dyn FusedStream<Item = Result<NetworkEvent<P>, BroadcastRecvError>> + Send>>,
}

impl<T: Message, P: Peer + 'static> ReceiveFromAll<T, P> {
    pub fn new<N: Network<PeerType = P> + ?Sized>(network: &N) -> Self {
        ReceiveFromAll {
            inner: stream::select_all(network.get_peers().iter().map(|peer| peer.receive::<T>())),
            event_stream: Box::pin(network.subscribe_events().into_stream().fuse()),
        }
    }
}

impl<T: Message, P: Peer> Stream for ReceiveFromAll<T, P> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.event_stream.poll_next_unpin(cx) {
                Poll::Pending => break,
                Poll::Ready(Some(Ok(NetworkEvent::PeerJoined(peer)))) => {
                    // We have a new peer to receive from.
                    self.inner.push(peer.receive::<T>())
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
        self.inner.poll_next_unpin(cx)
    }
}

impl<T: Message, P: Peer> FusedStream for ReceiveFromAll<T, P> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated() || self.event_stream.is_terminated()
    }
}

struct RequestResponseState<Res: ResponseMessage> {
    current_request_identifier: u32,
    responses: HashMap<u32, Sender<Res>>,
}

pub type ResponseTimeout = u32;

pub struct RequestResponse<P: Peer, Req: RequestMessage, Res: ResponseMessage> {
    peer: Arc<P>,
    state: Arc<Mutex<RequestResponseState<Res>>>,
    timeout: Duration,
    _req_type: PhantomData<Req>,
}

pub enum RequestError {
    Timeout,
    SendError(SendError),
    ReceiveError,
}

// Probably not really `Message` as types, but something that has a request identifier.
impl<P: Peer, Req: RequestMessage, Res: ResponseMessage + 'static> RequestResponse<P, Req, Res> {
    pub fn new(peer: Arc<P>, timeout: Duration) -> Self {
        let state = Arc::new(Mutex::new(RequestResponseState {
            current_request_identifier: 0,
            responses: Default::default(),
        }));

        // Poll stream and distribute messages to oneshot channels.
        let stream = peer.receive::<Res>();
        let weak_state = Arc::downgrade(&state);
        let weak_state2 = Weak::clone(&weak_state);
        // We only poll the stream while this struct still exists (as indicated by the weak ref).
        spawn(
            stream
                .take_while(move |_: &Res| future::ready(weak_state2.strong_count() > 0))
                .for_each(move |item: Res| {
                    if let Some(state) = weak_state.upgrade() {
                        let request_identifier = item.get_request_identifier();
                        let mut state = state.lock();
                        if let Some(sender) = state.responses.remove(&request_identifier) {
                            sender.send(item).ok();
                        }
                    }
                    future::ready(())
                }),
        );

        RequestResponse {
            peer,
            state,
            timeout,
            _req_type: PhantomData,
        }
    }

    pub async fn request(&self, mut request: Req) -> Result<Res, RequestError> {
        // Lock state, set identifier and send out request. Also add channel to the state.
        let mut state = self.state.lock();
        let request_identifier = state.current_request_identifier;
        state.current_request_identifier += 1;

        request.set_request_identifier(request_identifier);

        let (sender, receiver) = channel();
        state.responses.insert(request_identifier, sender);
        drop(state);

        // TODO: CloseType
        // If sending fails, remove channel and return error.
        if let Err(e) = self
            .peer
            .send_or_close(&request, |_| CloseReason::Other)
            .await
        {
            let mut state = self.state.lock();
            state.responses.remove(&request_identifier);
            return Err(RequestError::SendError(e));
        }

        // Now we only have to wait for the response.
        let response = timeout(self.timeout, receiver).await;

        // Lock state and remove channel on timeout.
        if let Err(_) = response {
            let mut state = self.state.lock();
            state.responses.remove(&request_identifier);
            return Err(RequestError::Timeout);
        }

        // Flatten response.
        response
            .ok()
            .map(|inner| inner.ok())
            .flatten()
            .ok_or(RequestError::ReceiveError)
    }
}
