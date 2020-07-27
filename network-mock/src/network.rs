use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::mpsc::unbounded;
use futures::{executor, future, FutureExt, Sink, SinkExt, Stream, StreamExt};
use parking_lot::RwLock;
use tokio::sync::broadcast::{
    channel as broadcast, Receiver as BroadcastReceiver, Sender as BroadcastSender,
};

use nimiq_network_interface::message::{peek_type, Message};
use nimiq_network_interface::network::{Network, NetworkEvent};
use nimiq_network_interface::peer::dispatch::{unbounded_dispatch, DispatchError};
use nimiq_network_interface::peer::{CloseReason, Peer, SendError};

pub type Channels =
    Arc<RwLock<HashMap<u64, Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>>>>;

pub struct MockPeer {
    id: u32,
    tx: Arc<RwLock<Option<Pin<Box<dyn Sink<Vec<u8>, Error = SendError> + Send + Sync>>>>>,
    channels: Channels,
    closed: Arc<AtomicBool>,
}

impl MockPeer {
    pub fn new(
        id: u32,
        tx: Pin<Box<dyn Sink<Vec<u8>, Error = SendError> + Send + Sync>>,
        rx: Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>,
    ) -> Self {
        let tx = Arc::new(RwLock::new(Some(tx)));
        let tx1 = Arc::clone(&tx);

        let channels: Channels = Arc::new(RwLock::new(HashMap::new()));
        let channels1 = Arc::clone(&channels);
        let channels2 = Arc::clone(&channels);

        let closed = Arc::new(AtomicBool::new(false));
        let closed1 = Arc::clone(&closed);

        tokio::spawn(
            rx
                .take_while(move |_| future::ready(!closed1.load(Ordering::Relaxed)))
                .for_each(move |msg: Vec<u8>| {
                    if let Ok(msg_type) = peek_type(&msg) {
                        let mut channels = channels1.write();
                        if let Some(channel) = channels.get_mut(&msg_type) {
                            if let Err(_) = executor::block_on(channel.send(msg)) {
                                channels.remove(&msg_type);
                            }
                        }
                    }
                    future::ready(())
                })
                .then(move |_| {
                    tx1.write().take();
                    channels2.write().clear();
                    future::ready(())
                }),
        );

        MockPeer {
            id,
            tx,
            channels,
            closed,
        }
    }
}

#[async_trait]
impl Peer for MockPeer {
    type Id = u32;

    fn id(&self) -> Self::Id {
        self.id
    }

    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError> {
        match self.tx.write().as_mut() {
            Some(tx) => {
                let mut serialized = Vec::with_capacity(msg.serialized_message_size());
                msg.serialize_message(&mut serialized)
                    .map_err(|e| SendError::Serialization(e))?;
                executor::block_on(tx.send(serialized))
            }
            None => Err(SendError::AlreadyClosed),
        }
    }

    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        let (tx, rx) = unbounded_dispatch();
        if let Some(_) = self.channels.write().insert(T::TYPE_ID, tx) {
            warn!("Receiver for message type {} already exists", T::TYPE_ID);
        }
        rx
    }

    async fn close(&self, ty: CloseReason) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

pub struct MockNetwork {
    peer_id: u32,
    peers: RwLock<HashMap<u32, Arc<MockPeer>>>,
    events: BroadcastSender<NetworkEvent<MockPeer>>,
}

impl MockNetwork {
    pub fn new(peer_id: u32) -> Self {
        let (tx, _rx) = broadcast(16);
        MockNetwork {
            peer_id,
            peers: RwLock::new(HashMap::new()),
            events: tx,
        }
    }

    pub fn connect(&self, other: &MockNetwork) {
        let (tx1, rx1) = unbounded();
        let (tx2, rx2) = unbounded();

        let peer1 = MockPeer::new(
            other.peer_id,
            Box::pin(tx1.sink_map_err(|_| SendError::AlreadyClosed)),
            rx2.boxed(),
        );
        let peer2 = MockPeer::new(
            self.peer_id,
            Box::pin(tx2.sink_map_err(|_| SendError::AlreadyClosed)),
            rx1.boxed(),
        );

        self.add_peer(Arc::new(peer1));
        other.add_peer(Arc::new(peer2));
    }

    fn add_peer(&self, peer: Arc<MockPeer>) {
        self.peers
            .write()
            .insert(peer.id(), Arc::clone(&peer))
            .map_or((), |_| panic!("Duplicate peer '{}'", peer.id()));
        self.events
            .send(NetworkEvent::PeerJoined(Arc::clone(&peer)));
    }
}

impl Network for MockNetwork {
    type PeerType = MockPeer;

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>> {
        self.peers
            .read()
            .values()
            .map(|peer| Arc::clone(peer))
            .collect()
    }

    fn get_peer(&self, peer_id: &<Self::PeerType as Peer>::Id) -> Option<Arc<Self::PeerType>> {
        self.peers.read().get(peer_id).map(|peer| Arc::clone(peer))
    }

    fn subscribe_events(&self) -> BroadcastReceiver<NetworkEvent<Self::PeerType>> {
        self.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use beserial::{Deserialize, Serialize};
    use futures::StreamExt;
    use nimiq_network_interface::message::Message;
    use nimiq_network_interface::network::{Network, NetworkEvent};
    use nimiq_network_interface::peer::Peer;

    use crate::network::MockNetwork;

    #[derive(Deserialize, Serialize)]
    struct TestMessage {
        id: u32,
    }
    impl Message for TestMessage {
        const TYPE_ID: u64 = 42;
    }

    #[tokio::test]
    async fn two_networks_can_connect() {
        let net1 = Arc::new(MockNetwork::new(1));
        let net2 = Arc::new(MockNetwork::new(2));

        let net1_1 = Arc::clone(&net1);
        let net2_1 = Arc::clone(&net2);
        tokio::spawn(async move {
            net1_1.connect(&net2_1);
        });

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();
        match join!(events1.next(), events2.next()) {
            (Some(Ok(NetworkEvent::PeerJoined(p1))), Some(Ok(NetworkEvent::PeerJoined(p2)))) => {
                assert_eq!(p1.id(), 2);
                assert_eq!(p2.id(), 1);
            }
            _ => panic!("Unexpected event"),
        };

        assert_eq!(net1.get_peers().len(), 1);
        assert_eq!(net1.get_peers().first().unwrap().id, 2);
        assert_eq!(net1.get_peer(&2).unwrap().id, 2);
        assert!(net1.get_peer(&1).is_none());

        assert_eq!(net2.get_peers().len(), 1);
        assert_eq!(net2.get_peers().first().unwrap().id, 1);
        assert_eq!(net2.get_peer(&1).unwrap().id, 1);
        assert!(net2.get_peer(&2).is_none());
    }

    #[tokio::test]
    async fn peers_can_talk_to_each_other() {
        let net1 = MockNetwork::new(1);
        let net2 = MockNetwork::new(2);
        net1.connect(&net2);

        let peer1 = net2.get_peer(&1).unwrap();
        tokio::spawn(async move {
            peer1
                .send(&TestMessage { id: 4711 })
                .await
                .expect("Send failed");
        });

        let peer2 = net1.get_peer(&2).unwrap();
        let msg = peer2
            .receive::<TestMessage>()
            .next()
            .await
            .expect("Message expected");

        assert_eq!(msg.id, 4711);
    }
}
