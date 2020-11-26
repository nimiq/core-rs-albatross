use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::channel::oneshot::{channel as oneshot, Sender as OneshotSender};
use futures::task::{noop_waker_ref, Context, Poll};
use futures::{executor, future, FutureExt, Sink, SinkExt, Stream, StreamExt};
use parking_lot::{Mutex, RwLock};
use tokio::sync::broadcast;
use thiserror::Error;

use beserial::{Serialize, Deserialize};
use nimiq_network_interface::message::{peek_type, Message};
use nimiq_network_interface::network::{Network, NetworkEvent, Topic};
use nimiq_network_interface::peer::dispatch::{unbounded_dispatch, DispatchError};
use nimiq_network_interface::peer::{CloseReason, Peer, SendError, RequestResponse};

pub type Channels = Arc<RwLock<HashMap<u64, Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>>>>;


#[derive(Debug, Error)]
pub enum MockNetworkError {

}

pub struct MockPeer {
    id: usize,
    tx: Arc<Mutex<Option<Pin<Box<dyn Sink<Vec<u8>, Error = SendError> + Send + Sync>>>>>,
    channels: Channels,
    close_tx: Mutex<Option<OneshotSender<CloseReason>>>,
}

impl MockPeer {
    pub fn new(
        id: usize,
        tx: Pin<Box<dyn Sink<Vec<u8>, Error = SendError> + Send + Sync>>,
        rx: Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>,
        mut network_close_tx: UnboundedSender<usize>,
    ) -> Self {
        let tx = Arc::new(Mutex::new(Some(tx)));
        let tx1 = Arc::clone(&tx);

        let channels: Channels = Arc::new(RwLock::new(HashMap::new()));
        let channels1 = Arc::clone(&channels);
        let channels2 = Arc::clone(&channels);

        // Setup message dispatcher to distribute incoming messages to message-type specific channels.
        let dispatcher = rx.for_each(move |msg: Vec<u8>| {
            if let Ok(msg_type) = peek_type(&msg) {
                let mut channels = channels1.write();
                if let Some(channel) = channels.get_mut(&msg_type) {
                    if executor::block_on(channel.send(msg)).is_err() {
                        channels.remove(&msg_type);
                    }
                }
            }
            future::ready(())
        });
        // Oneshot channel to terminate the dispatcher.
        let (close_tx, close_rx) = oneshot::<CloseReason>();

        let select = future::select(dispatcher, close_rx).then(move |_| async move {
            // Drop all senders. tx1 might have already been dropped if we closed this connection.
            tx1.lock().take();
            channels2.write().clear();

            // Notify network that peer disconnected.
            network_close_tx.send(id).await
        });
        tokio::spawn(select);

        MockPeer {
            id,
            tx,
            channels,
            close_tx: Mutex::new(Some(close_tx)),
        }
    }
}

impl Hash for MockPeer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

impl PartialEq for MockPeer {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}

impl Eq for MockPeer {}

#[async_trait]
impl Peer for MockPeer {
    type Id = usize;
    type Error = MockNetworkError;

    fn id(&self) -> Self::Id {
        self.id
    }

    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError> {
        match self.tx.lock().as_mut() {
            Some(tx) => {
                let mut serialized = Vec::with_capacity(msg.serialized_message_size());
                msg.serialize_message(&mut serialized).map_err(SendError::Serialization)?;
                executor::block_on(tx.send(serialized))
            }
            None => Err(SendError::AlreadyClosed),
        }
    }

    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        let (tx, rx) = unbounded_dispatch();
        let mut channels = self.channels.write();
        // Check if receiver has been dropped if one already exists.
        if let Some(channel) = channels.get_mut(&T::TYPE_ID) {
            let mut cx = Context::from_waker(noop_waker_ref());
            // We expect the sink to return an error because the receiver is dropped.
            // If not, we would overwrite an active receive.
            match channel.as_mut().poll_ready(&mut cx) {
                Poll::Ready(Err(_)) => {}
                _ => panic!("Receiver for message type {} already exists", T::TYPE_ID),
            }
        }
        channels.insert(T::TYPE_ID, tx);

        rx
    }

    fn close(&self, ty: CloseReason) {
        if let Some(close_tx) = self.close_tx.lock().take() {
            close_tx.send(ty).expect("Close failed");

            // Eagerly drop sender to stop sending messages immediately.
            self.tx.lock().take();
        }
    }

    async fn request<R: RequestResponse>(&self, _request: &<R as RequestResponse>::Request) -> Result<R::Response, Self::Error> {
        unimplemented!()
    }

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> {
        unimplemented!()
    }
}

pub struct MockNetwork {
    peer_id: usize,
    peers: Arc<RwLock<HashMap<usize, Arc<MockPeer>>>>,
    event_tx: broadcast::Sender<NetworkEvent<MockPeer>>,
    close_tx: UnboundedSender<usize>,
}

impl MockNetwork {
    pub fn new(peer_id: usize) -> Self {
        let peers = Arc::new(RwLock::new(HashMap::new()));
        let (event_tx, _rx) = broadcast::channel(256);
        let (close_tx, close_rx) = unbounded();

        let peers1 = Arc::clone(&peers);
        let event_tx1 = event_tx.clone();
        tokio::spawn(close_rx.for_each(move |peer_id| {
            if let Some(peer) = peers1.write().remove(&peer_id) {
                event_tx1.send(NetworkEvent::PeerLeft(peer)).ok();
            }
            future::ready(())
        }));

        MockNetwork {
            peer_id,
            peers,
            event_tx,
            close_tx,
        }
    }

    pub fn connect(&self, other: &MockNetwork) {
        let (tx1, rx1) = unbounded();
        let (tx2, rx2) = unbounded();

        let peer1 = MockPeer::new(
            other.peer_id,
            Box::pin(tx1.sink_map_err(|_| SendError::AlreadyClosed)),
            rx2.boxed(),
            self.close_tx.clone(),
        );
        let peer2 = MockPeer::new(
            self.peer_id,
            Box::pin(tx2.sink_map_err(|_| SendError::AlreadyClosed)),
            rx1.boxed(),
            other.close_tx.clone(),
        );

        self.add_peer(Arc::new(peer1));
        other.add_peer(Arc::new(peer2));
    }

    fn add_peer(&self, peer: Arc<MockPeer>) {
        self.peers
            .write()
            .insert(peer.id(), Arc::clone(&peer))
            .map_or((), |_| panic!("Duplicate peer '{}'", peer.id()));
        self.event_tx.send(NetworkEvent::PeerJoined(Arc::clone(&peer))).ok();
    }

    pub fn disconnect(&self) {
        for (_, peer) in self.peers.write().drain() {
            self.event_tx.send(NetworkEvent::PeerLeft(peer)).ok();
        }
    }
}

#[async_trait]
impl Network for MockNetwork {
    type PeerType = MockPeer;
    type Error = MockNetworkError;

    fn get_peer_updates(&self) -> (Vec<Arc<MockPeer>>, broadcast::Receiver<NetworkEvent<MockPeer>>) {
        unimplemented!()
    }

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>> {
        self.peers.read().values().map(|peer| Arc::clone(peer)).collect()
    }

    fn get_peer(&self, peer_id: <Self::PeerType as Peer>::Id) -> Option<Arc<Self::PeerType>> {
        self.peers.read().get(&peer_id).cloned()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<NetworkEvent<Self::PeerType>> {
        self.event_tx.subscribe()
    }

    async fn subscribe<T>(_topic: &T) -> Box<dyn Stream<Item = (T::Item, Self::PeerType)> + Send>
        where
            T: Topic + Sync,
    {
        unimplemented!()
    }

    async fn publish<T>(_topic: &T, _item: <T as Topic>::Item)
        where
            T: Topic + Sync,
    {
        unimplemented!()
    }

    async fn dht_get<K, V>(&self, _k: &K) -> Result<V, Self::Error>
        where
            K: AsRef<[u8]> + Send + Sync,
            V: Deserialize + Send + Sync,
    {
        unimplemented!()
    }

    async fn dht_put<K, V>(&self, _k: &K, _v: &V) -> Result<(), Self::Error>
        where
            K: AsRef<[u8]> + Send + Sync,
            V: Serialize + Send + Sync,
    {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures::{join, StreamExt};

    use beserial::{Deserialize, Serialize};
    use nimiq_network_interface::message::{Message, RequestMessage, ResponseMessage};
    use nimiq_network_interface::network::{Network, NetworkEvent};
    use nimiq_network_interface::peer::{CloseReason, Peer};
    use nimiq_network_interface::request_response::RequestResponse;

    use crate::network::MockNetwork;

    #[derive(Debug, Deserialize, Serialize)]
    struct TestMessage {
        id: u32,
    }
    impl Message for TestMessage {
        const TYPE_ID: u64 = 42;
    }

    impl RequestMessage for TestMessage {
        fn set_request_identifier(&mut self, request_identifier: u32) {
            self.id = request_identifier;
        }
    }

    impl ResponseMessage for TestMessage {
        fn get_request_identifier(&self) -> u32 {
            self.id
        }
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
        assert_eq!(net1.get_peer(2).unwrap().id, 2);
        assert!(net1.get_peer(1).is_none());

        assert_eq!(net2.get_peers().len(), 1);
        assert_eq!(net2.get_peers().first().unwrap().id, 1);
        assert_eq!(net2.get_peer(1).unwrap().id, 1);
        assert!(net2.get_peer(2).is_none());
    }

    #[tokio::test]
    async fn peers_can_talk_to_each_other() {
        let net1 = MockNetwork::new(1);
        let net2 = MockNetwork::new(2);
        net1.connect(&net2);

        let peer1 = net2.get_peer(1).unwrap();
        tokio::spawn(async move {
            peer1.send(&TestMessage { id: 4711 }).await.expect("Send failed");
        });

        let peer2 = net1.get_peer(2).unwrap();
        let msg = peer2.receive::<TestMessage>().next().await.expect("Message expected");
        assert_eq!(msg.id, 4711);
    }

    #[tokio::test]
    async fn connections_are_properly_closed() {
        let net1 = MockNetwork::new(1);
        let net2 = MockNetwork::new(2);
        net1.connect(&net2);

        assert_eq!(net1.get_peers().len(), 1);
        assert_eq!(net2.get_peers().len(), 1);

        let peer2 = net1.get_peer(2).unwrap();
        tokio::spawn(async move {
            peer2.close(CloseReason::Other);
            // This message should not arrive.
            peer2.send(&TestMessage { id: 6969 }).await.expect_err("Message didn't fail");
        });

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();
        let peer1 = net2.get_peer(1).unwrap();
        let mut no_msg = peer1.receive::<TestMessage>();
        match join!(events1.next(), events2.next(), no_msg.next()) {
            (Some(Ok(NetworkEvent::PeerLeft(p1))), Some(Ok(NetworkEvent::PeerLeft(p2))), None) => {
                assert_eq!(p1.id(), 2);
                assert_eq!(p2.id(), 1);
            }
            _ => panic!("Unexpected event"),
        }

        assert_eq!(net1.get_peers().len(), 0);
        assert_eq!(net2.get_peers().len(), 0);
    }

    #[tokio::test]
    async fn request_response() {
        let net1 = MockNetwork::new(1);
        let net2 = MockNetwork::new(2);
        net1.connect(&net2);

        assert_eq!(net1.get_peers().len(), 1);
        assert_eq!(net2.get_peers().len(), 1);

        let mut stream = net2.receive_from_all::<TestMessage>();
        // Relay messages back.
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                peer.send(&msg).await.unwrap();
            }
        });

        let peer2 = net1.get_peer(2).unwrap();
        let requests = RequestResponse::<_, TestMessage, TestMessage>::new(peer2, Duration::new(1, 0));

        let msg = TestMessage { id: 0 };
        let response = requests.request(msg).await.expect("TestMessage expected");
        assert_eq!(response.id, 0);
    }
}
