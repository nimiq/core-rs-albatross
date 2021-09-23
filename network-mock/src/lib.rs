#[macro_use]
extern crate beserial_derive;

mod hub;
mod network;
mod peer;

use beserial::{Deserialize, Serialize};
use derive_more::{Display, From, Into};

pub use hub::MockHub;
pub use network::{MockId, MockNetwork};
pub use peer::MockPeer;

/// The address of a MockNetwork or a peer thereof. Peer IDs are always equal to their respective address, thus these
/// can be converted between each other.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Display, From, Into)]
pub struct MockAddress(u64);

/// The peer ID of a MockNetwork or a peer thereof. Peer IDs are always equal to their respective address, thus these
/// can be converted between each other.
#[derive(
    Copy,
    Clone,
    Debug,
    Deserialize,
    Serialize,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Display,
    From,
    Into,
)]
pub struct MockPeerId(u64);

impl From<MockAddress> for MockPeerId {
    fn from(address: MockAddress) -> Self {
        Self(address.0)
    }
}

impl From<MockPeerId> for MockAddress {
    fn from(peer_id: MockPeerId) -> Self {
        Self(peer_id.0)
    }
}

pub async fn create_mock_validator_network(n: usize, dial: bool) -> Vec<MockNetwork> {
    let mut hub = MockHub::default();

    let networks = (0..n)
        .map(|_| hub.new_network())
        .collect::<Vec<MockNetwork>>();

    if n > 0 && dial {
        // Peers i>0 dial peer 0
        for net in &networks[1..] {
            net.dial_mock(&networks[0]);
        }
    }

    networks
}

#[cfg(test)]
pub mod tests {
    use futures::{Stream, StreamExt};
    use tokio_stream::wrappers::BroadcastStream;

    use beserial::{Deserialize, Serialize};
    use nimiq_network_interface::{
        message::Message,
        network::{Network, NetworkEvent, Topic},
        peer::Peer,
    };

    use super::network::MockNetworkError;
    use super::{MockHub, MockPeer, MockPeerId};

    pub async fn assert_peer_joined(
        events: &mut BroadcastStream<NetworkEvent<MockPeer>>,
        peer_id: MockPeerId,
    ) {
        if let Some(Ok(NetworkEvent::PeerJoined(peer))) = events.next().await {
            assert_eq!(peer.id(), peer_id);
        } else {
            panic!("Expected PeerJoined event with id={}", peer_id);
        }
    }

    pub async fn assert_peer_left(
        events: &mut BroadcastStream<NetworkEvent<MockPeer>>,
        peer_id: MockPeerId,
    ) {
        if let Some(Ok(NetworkEvent::PeerLeft(peer))) = events.next().await {
            assert_eq!(peer.id(), peer_id);
        } else {
            panic!("Expected PeerLeft event with id={}", peer_id);
        }
    }

    #[tokio::test]
    async fn test_peer_list() {
        let mut hub = MockHub::default();

        let net1 = hub.new_network();
        let net2 = hub.new_network();
        let net3 = hub.new_network();
        let net4 = hub.new_network();

        // net1 and net2 already connected
        net1.dial_mock(&net2);

        // event stream starts here
        let (peers, mut events) = net1.get_peer_updates();

        // net1 dials net3
        net1.dial_mock(&net3);

        // net1 is dialed by net3
        net4.dial_mock(&net1);

        // net1 and net2 already connected
        assert_eq!(peers.len(), 1);
        assert_eq!(peers.get(0).unwrap().id(), net2.peer_id());

        assert_peer_joined(&mut events, net3.peer_id()).await;
        assert_peer_joined(&mut events, net4.peer_id()).await;

        // test get_peers
        let mut peer_ids = net1
            .get_peers()
            .into_iter()
            .map(|peer| peer.id())
            .collect::<Vec<MockPeerId>>();
        peer_ids.sort();
        let mut expected_peer_ids = vec![net2.peer_id(), net3.peer_id(), net4.peer_id()];
        expected_peer_ids.sort();

        assert_eq!(peer_ids, expected_peer_ids);
    }

    // Copied straight from nimiq_network_libp2p::network

    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    pub struct TestRecord {
        x: i32,
    }

    #[tokio::test]
    async fn dht_put_and_get() {
        let mut hub = MockHub::new();
        let net1 = hub.new_network();
        let net2 = hub.new_network();
        net1.dial_mock(&net2);

        let put_record = TestRecord { x: 420 };

        net1.dht_put(b"foo", &put_record).await.unwrap();

        let fetched_record = net2.dht_get::<_, TestRecord>(b"foo").await.unwrap();

        assert_eq!(fetched_record, Some(put_record));
    }

    pub struct TestTopic;

    impl Topic for TestTopic {
        type Item = TestRecord;

        const BUFFER_SIZE: usize = 8;
        const NAME: &'static str = "hello_world";
        const VALIDATE: bool = false;
    }

    fn consume_stream<T: std::fmt::Debug>(
        mut stream: impl Stream<Item = T> + Unpin + Send + 'static,
    ) {
        tokio::spawn(async move { while stream.next().await.is_some() {} });
    }

    #[tokio::test]
    async fn test_gossipsub() {
        let mut hub = MockHub::new();
        let net1 = hub.new_network();
        let net2 = hub.new_network();
        net1.dial_mock(&net2);

        let test_message = TestRecord { x: 42 };

        let mut messages = net1.subscribe::<TestTopic>().await.unwrap();
        consume_stream(net2.subscribe::<TestTopic>().await.unwrap());

        net2.publish::<TestTopic>(test_message.clone())
            .await
            .unwrap();

        let (received_message, _peer) = messages.next().await.unwrap();
        log::info!("Received Gossipsub message: {:?}", received_message);

        assert_eq!(received_message, test_message);

        // Assert that we can unsubscribe from topics
        assert_eq!(Ok(()), net1.unsubscribe::<TestTopic>().await);
        assert_eq!(
            Err(MockNetworkError::AlreadyUnsubscribed(TestTopic::NAME)),
            net1.unsubscribe::<TestTopic>().await
        );
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct TestMessage {
        id: u32,
    }

    impl Message for TestMessage {
        const TYPE_ID: u64 = 42;
    }

    #[tokio::test]
    async fn both_peers_can_talk_with_each_other() {
        let mut hub = MockHub::new();
        let net1 = hub.new_network();
        let net2 = hub.new_network();
        net1.dial_mock(&net2);

        let peer2 = net1.get_peer(net2.peer_id()).unwrap();
        let peer1 = net2.get_peer(net1.peer_id()).unwrap();

        let mut in1 = peer1.receive::<TestMessage>();
        let mut in2 = peer2.receive::<TestMessage>();

        peer1.send(&TestMessage { id: 1337 }).await.unwrap();
        peer2.send(&TestMessage { id: 420 }).await.unwrap();

        let msg1 = in2.next().await.unwrap();
        let msg2 = in1.next().await.unwrap();

        assert_eq!(msg1.id, 1337);
        assert_eq!(msg2.id, 420);
    }
}
