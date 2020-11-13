use std::collections::HashMap;
use std::hash::Hash;
use std::hash::Hasher;
use std::pin::Pin;

use async_trait::async_trait;
use futures::channel::mpsc;
use futures::lock::Mutex as AsyncMutex;
use futures::task::{noop_waker_ref, Context, Poll};
use futures::{executor, Sink, SinkExt, Stream, TryFutureExt, Future};
use libp2p::{
    swarm::NegotiatedSubstream,
    PeerId
};
use parking_lot::Mutex;

use beserial::SerializingError;
use nimiq_network_interface::peer::{
    dispatch::{unbounded_dispatch, DispatchError},
    CloseReason, Peer as PeerInterface, SendError, RequestResponse,
};
use nimiq_network_interface::{message, message::Message};

use super::dispatch::{MessageReceiver, MessageSender};
use crate::network::NetworkError;

#[derive(Debug)]
pub(crate) enum PeerAction {
    Message(PeerId, Vec<u8>),
    Close(PeerId),
}

pub struct Peer {
    pub id: PeerId,

    inbound: MessageReceiver,
    outbound: MessageSender<NegotiatedSubstream>,

    //tx: AsyncMutex<mpsc::Sender<PeerAction>>,
    //channels: Mutex<HashMap<u64, Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>>>,
    //pub banned: bool,
}

impl Peer {
    pub(crate) fn new(id: PeerId, inbound: MessageReceiver, outbound: MessageSender<NegotiatedSubstream>) -> Self {
        Self {
            id,
            inbound,
            outbound,
        }
    }
}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Peer")
            .field("peer_id", &self.id())
            .finish()
    }
}

impl PartialEq for Peer {
    fn eq(&self, other: &Peer) -> bool {
        self.id == other.id
    }
}

impl Eq for Peer {}

impl Hash for Peer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

#[async_trait]
impl PeerInterface for Peer {
    type Id = PeerId;
    type Error = NetworkError;

    fn id(&self) -> Self::Id {
        self.id.clone()
    }

    async fn send<M: Message>(&self, msg: &M) -> Result<(), SendError> {
        Ok(self.outbound.send(msg).await?)
    }

    fn receive<M: Message>(&self) -> Pin<Box<dyn Stream<Item = M> + Send>> {
        Box::pin(self.inbound.receive())
    }

    async fn close(&self, _ty: CloseReason) {

    }

    async fn request<R: RequestResponse>(&self, request: &<R as RequestResponse>::Request) -> Result<R::Response, Self::Error> {
        unimplemented!()
    }

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> {
        unimplemented!()
    }
}
