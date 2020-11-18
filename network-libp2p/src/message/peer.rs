use std::{
    hash::{Hash, Hasher},
    pin::Pin,
};

use async_trait::async_trait;
use futures::{
    channel::oneshot,
    Stream,
};
use libp2p::{
    swarm::NegotiatedSubstream,
    PeerId
};
use parking_lot::Mutex;

use nimiq_network_interface::peer::{
    CloseReason, Peer as PeerInterface, SendError, RequestResponse,
};
use nimiq_network_interface::message::Message;

use super::dispatch::MessageDispatch;
use crate::network::NetworkError;

#[derive(Debug)]
pub(crate) enum PeerAction {
    Message(PeerId, Vec<u8>),
    Close(PeerId),
}

pub struct Peer {
    pub id: PeerId,

    socket: MessageDispatch<NegotiatedSubstream>,

    //tx: AsyncMutex<mpsc::Sender<PeerAction>>,
    //channels: Mutex<HashMap<u64, Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>>>,
    //pub banned: bool,
}

impl Peer {
    pub(crate) fn new(id: PeerId, socket: MessageDispatch<NegotiatedSubstream>) -> Self {
        Self {
            id,
            socket,
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

    async fn send<M: Message>(&self, message: &M) -> Result<(), SendError> {
        Ok(self.socket.outbound.send(message).await?)
    }

    fn receive<M: Message>(&self) -> Pin<Box<dyn Stream<Item = M> + Send>> {
        Box::pin(self.socket.inbound.receive())
    }

    async fn close(&self, _ty: CloseReason) {

    }

    async fn request<R: RequestResponse>(&self, _request: &<R as RequestResponse>::Request) -> Result<R::Response, Self::Error> {
        unimplemented!()
    }

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> {
        unimplemented!()
    }
}
