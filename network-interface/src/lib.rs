use async_trait::async_trait;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use futures::{
    channel::oneshot::{channel, Sender},
    future,
    stream::select_all,
    Stream, StreamExt,
};
use nimiq_network_primitives::address::PeerId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::{task::spawn, time::timeout};

pub trait Message: Serialize + Deserialize + Send {
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
pub trait Peer {
    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError>;
    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>>;
    fn close(&self, ty: CloseReason);
}

#[async_trait]
pub trait Network {
    type PeerType: Peer;

    fn get_peers(&self) -> &[Arc<Self::PeerType>];
    fn get_peer(&self, peer_id: PeerId) -> &Arc<Self::PeerType>;

    //    async fn broadcast<T: Message>(&self, msg: &T) {
    //        self.get_peers().iter().map(|peer| peer.send(msg).) // TODO: Close connection
    //    }
    //
    //    // TODO: What if new peers join?
    //    fn receive_from_all<'a, T: Message + 'a>(&self) -> Pin<Box<dyn Stream<Item = T> + Send + 'a>> {
    //        select_all(self.get_peers().iter().map(|peer| peer.receive::<T>())).boxed()
    //    }
}

// .next() To get next item of stream.

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

    pub async fn request(&self, mut request: Req) -> Option<Res> {
        // Lock state, set identifier and send out request. Also add channel to the state.
        let mut state = self.state.lock();
        let request_identifier = state.current_request_identifier;
        state.current_request_identifier += 1;

        request.set_request_identifier(request_identifier);
        self.peer.send(&request);

        let (sender, receiver) = channel();
        state.responses.insert(request_identifier, sender);
        drop(state);

        // Now we only have to wait for the response.
        let response = timeout(self.timeout, receiver).await;

        // Lock state and remove channel on timeout.
        if let Err(_) = response {
            let mut state = self.state.lock();
            state.responses.remove(&request_identifier);
        }

        // Flatten response.
        response.ok().map(|inner| inner.ok()).flatten()
    }
}
