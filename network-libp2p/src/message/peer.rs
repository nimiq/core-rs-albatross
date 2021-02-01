use std::{
    hash::{Hash, Hasher},
    pin::Pin,
    task::{Context, Poll},
    sync::Arc,
};

use async_trait::async_trait;
use futures::{
    channel::oneshot,
    stream::{Stream, StreamExt},
};
use libp2p::{swarm::NegotiatedSubstream, PeerId};
use parking_lot::Mutex;

use nimiq_network_interface::message::Message;
use nimiq_network_interface::peer::{CloseReason, Peer as PeerInterface, RequestResponse, SendError};

use super::dispatch::{MessageDispatch, SendMessage};
use crate::{
    network::NetworkError,
    codecs::typed::Error,
};

pub struct Peer {
    pub id: PeerId,

    pub(crate) dispatch: Arc<Mutex<MessageDispatch<NegotiatedSubstream>>>,

    /// Channel used to pass the close reason the the network handler.
    close_tx: Mutex<Option<oneshot::Sender<CloseReason>>>,
}

impl Peer {
    pub fn new(id: PeerId, dispatch: MessageDispatch<NegotiatedSubstream>, close_tx: oneshot::Sender<CloseReason>) -> Self {
        Self {
            id,
            dispatch: Arc::new(Mutex::new(dispatch)),
            close_tx: Mutex::new(Some(close_tx)),
        }
    }

    /// Polls the underlying dispatch's inbound stream by first trying to acquire the mutex. If it's not availble,
    /// this will return `Poll::Pending` and make sure that the task is woken up, once the mutex was released.
    pub fn poll_inbound(self: &Arc<Peer>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let p = self.dispatch.lock().poll_inbound(cx, self);

        log::trace!("Polled socket: peer={:?}: {:?}", self.id, p);

        p
    }

    pub fn poll_close(&self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.dispatch.lock().poll_close(cx)
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
        SendMessage::new(Arc::clone(&self.dispatch), message)
            .await
            .map_err(|e| e.into())
    }

    // TODO: Make this a stream of Result<M, Error>
    fn receive<M: Message>(&self) -> Pin<Box<dyn Stream<Item = M> + Send>> {
        self.dispatch
            .lock()
            .receive()
            .boxed()
    }

    fn close(&self, reason: CloseReason) {
        // TODO: I think we must poll_close on the underlying socket

        log::debug!("Peer::close: reason={:?}", reason);

        let close_tx_opt = self.close_tx.lock().take();

        if let Some(close_tx) = close_tx_opt {
            if close_tx.send(reason).is_err() {
                log::error!("The receiver for Peer::close was already dropped.");
            }
        } else {
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
