use std::{
    sync::Arc,
    pin::Pin,
    hash::{Hasher, Hash},
    io::Cursor,
};

use futures::{
    channel::mpsc,
    stream::{StreamExt, Stream},
    sink::SinkExt,
};
use parking_lot::Mutex;
use async_trait::async_trait;

use nimiq_network_interface::message::Message;
use nimiq_network_interface::peer::{Peer, CloseReason, RequestResponse, SendError};

use crate::{
    hub::{MockHubInner, SenderKey},
    network::MockNetworkError,
    MockPeerId, MockAddress,
};


#[derive(Debug)]
pub struct MockPeer {
    /// The address of the network that sees this peer
    pub(crate) network_address: MockAddress,

    /// The peer's peer ID
    pub(crate) peer_id: MockPeerId,

    pub(crate) hub: Arc<Mutex<MockHubInner>>,
}

#[async_trait]
impl Peer for MockPeer {
    type Id = MockPeerId;
    type Error = MockNetworkError;

    fn id(&self) -> MockPeerId {
        self.peer_id
    }

    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError> {
        let k = SenderKey {
            network_recipient: self.peer_id.into(),
            sender_peer: self.network_address.into(),
            message_type: T::TYPE_ID,
        };

        let mut sender = {
            let hub = self.hub.lock();

            if let Some(sender) = hub.network_senders.get(&k) {
                sender.clone()
            }
            else {
                return Ok(())
            }
        };

        let mut data = vec![];
        msg.serialize_message(&mut data).unwrap();

        sender.send(data).await.map_err(|_| SendError::AlreadyClosed)?;

        Ok(())
    }

    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        let mut hub = self.hub.lock();

        let (tx, rx) = mpsc::channel(16);

        hub.network_senders
            .insert(SenderKey {
                network_recipient: self.network_address,
                sender_peer: self.peer_id,
                message_type: T::TYPE_ID,
            }, tx);

        rx.filter_map(|data| async move {
            match T::deserialize_message(&mut Cursor::new(data)) {
                Ok(message) => Some(message),
                Err(e) => {
                    // TODO: Give MockHub a config, so that we can panic here if that's what the test wants to do.
                    log::warn!("Failed to deserialize message: {}", e);
                    None
                }
            }
        }).boxed()
    }

    fn close(&self, _ty: CloseReason) {
        let mut hub = self.hub.lock();

        // Drops senders and thus the receiver stream will end
        hub.network_senders.retain(|k, _sender| {
            k.network_recipient != self.network_address
        });
    }

    async fn request<R: RequestResponse>(&self, _request: &<R as RequestResponse>::Request) -> Result<R::Response, Self::Error> {
        unimplemented!()
    }

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> {
        unimplemented!()
    }
}

impl PartialEq for MockPeer {
    fn eq(&self, other: &Self) -> bool {
        self.peer_id == other.peer_id
    }
}

impl Eq for MockPeer {}

impl Hash for MockPeer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.peer_id.hash(state);
    }
}
