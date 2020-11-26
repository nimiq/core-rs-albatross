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


pub struct Peer {
    pub id: PeerId,

    pub(crate) socket: MessageDispatch<NegotiatedSubstream>,

    close_tx: Mutex<Option<oneshot::Sender<CloseReason>>>,
}

impl Peer {
    pub fn new(id: PeerId, socket: MessageDispatch<NegotiatedSubstream>, close_tx: oneshot::Sender<CloseReason>) -> Self {
        Self {
            id,
            socket,
            close_tx: Mutex::new(Some(close_tx)),
        }
    }


}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut debug = f.debug_struct("Peer");

        debug.field("peer_id", &self.id());

        if self.close_tx.lock().is_none() {
            debug.field("closed", &true);
        }

        debug.finish()
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

    fn close(&self, reason: CloseReason) {
        log::debug!("Peer::close: reason={:?}", reason);

        let close_tx_opt = self.close_tx.lock().take();

        if let Some(close_tx) = close_tx_opt {
            if let Err(_) = close_tx.send(reason) {
                log::error!("The receiver for Peer::close was already dropped.");
            }
        }
        else {
            log::error!("Peer is already closed");
        }
    }

    async fn request<R: RequestResponse>(&self, _request: &<R as RequestResponse>::Request) -> Result<R::Response, Self::Error> {
        unimplemented!()
    }

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> {
        unimplemented!()
    }
}
