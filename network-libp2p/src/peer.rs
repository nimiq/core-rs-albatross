use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use futures::channel::mpsc;
use futures::lock::Mutex as AsyncMutex;
use futures::task::{noop_waker_ref, Context, Poll};
use futures::{executor, Sink, SinkExt, Stream, TryFutureExt};
use libp2p::PeerId;

use network_interface::peer::{
    dispatch::{unbounded_dispatch, DispatchError},
    CloseReason, Peer as PeerInterface, SendError,
};
use network_interface::{message, message::Message};
use parking_lot::Mutex;

#[derive(Debug)]
pub(crate) enum PeerAction {
    Message(PeerId, Vec<u8>),
    Close(PeerId),
}

pub struct Peer {
    pub id: PeerId,
    tx: AsyncMutex<mpsc::Sender<PeerAction>>,
    channels: Mutex<HashMap<u64, Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>>>,
}

impl Peer {
    pub(crate) fn new(id: PeerId, tx: mpsc::Sender<PeerAction>) -> Self {
        Self {
            id,
            tx: AsyncMutex::new(tx),
            channels: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn dispatch_inbound_msg(&self, msg: Vec<u8>) {
        if let Ok(msg_type) = message::peek_type(&msg) {
            let mut channels = self.channels.lock();
            if let Some(channel) = channels.get_mut(&msg_type) {
                // TODO don't block here
                if let Err(_) = executor::block_on(channel.send(msg)) {
                    channels.remove(&msg_type);
                }
            }
        }
    }
}

#[async_trait]
impl PeerInterface for Peer {
    type Id = PeerId;

    fn id(&self) -> Self::Id {
        self.id.clone()
    }

    async fn send<M: Message>(&self, msg: &M) -> Result<(), SendError> {
        let mut serialized = Vec::with_capacity(msg.serialized_message_size());
        msg.serialize_message(&mut serialized)
            .map_err(|e| SendError::Serialization(e))?;

        self.tx
            .lock()
            .await
            .send(PeerAction::Message(self.id.clone(), serialized))
            // Error should never happen as long as the NetworkBehaviour exists which holds on to
            // the receiving end of this channel.
            .map_err(|_| SendError::AlreadyClosed)
            .await
    }

    fn receive<M: Message>(&self) -> Pin<Box<dyn Stream<Item = M> + Send>> {
        let (tx, rx) = unbounded_dispatch();

        // Check if receiver has been dropped if one already exists.
        let mut channels = self.channels.lock();
        if let Some(mut channel) = channels.insert(M::TYPE_ID, tx) {
            // We expect the sink to return an error because the receiver is dropped.
            // If not, we would overwrite an active receive.
            let mut cx = Context::from_waker(noop_waker_ref());
            match channel.poll_ready_unpin(&mut cx) {
                Poll::Ready(Err(_)) => {}
                _ => panic!("Receiver for message type {} already exists", M::TYPE_ID),
            }
        }

        rx
    }

    async fn close(&self, ty: CloseReason) {
        println!("Closing connection to {} (reason {:?})", self.id, ty);

        self.tx
            .lock()
            .await
            .send(PeerAction::Close(self.id.clone()))
            .await
            .expect("Failed to dispatch close action");
    }
}
