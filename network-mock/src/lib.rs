mod hub;
mod network;
mod observable_hash_map;

use derive_more::{Display, From, Into};
pub use hub::MockHub;
pub use network::{MockId, MockNetwork};
use nimiq_network_interface::{multiaddr, Multiaddr};
pub use observable_hash_map::ObservableHashMap;
use serde::{Deserialize, Serialize};

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
pub struct MockPeerId(pub u64);

impl From<MockAddress> for MockPeerId {
    fn from(address: MockAddress) -> Self {
        Self(address.0)
    }
}

impl From<MockAddress> for Multiaddr {
    fn from(address: MockAddress) -> Self {
        multiaddr!(Memory(address.0))
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
    use nimiq_keys::{KeyPair, SecureGenerate};
    use nimiq_network_interface::network::{Network, NetworkEvent, SubscribeEvents, Topic};
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng::test_rng;
    use nimiq_utils::{spawn::spawn, tagged_signing::TaggedSignable};
    use serde::{Deserialize, Serialize};

    use super::{network::MockNetworkError, MockHub, MockPeerId};

    pub async fn assert_peer_joined(
        events: &mut SubscribeEvents<MockPeerId>,
        expected_peer_id: MockPeerId,
    ) {
        if let Some(Ok(NetworkEvent::PeerJoined(peer_id, _peer_info))) = events.next().await {
            assert_eq!(peer_id, expected_peer_id);
        } else {
            panic!("Expected PeerJoined event with id={expected_peer_id}");
        }
    }

    pub async fn assert_peer_left(
        events: &mut SubscribeEvents<MockPeerId>,
        expected_peer_id: MockPeerId,
    ) {
        if let Some(Ok(NetworkEvent::PeerLeft(peer_id))) = events.next().await {
            assert_eq!(peer_id, expected_peer_id);
        } else {
            panic!("Expected PeerLeft event with id={expected_peer_id}");
        }
    }

    #[test(tokio::test)]
    async fn test_peer_list() {
        let mut hub = MockHub::default();

        let net1 = hub.new_network();
        let net2 = hub.new_network();
        let net3 = hub.new_network();
        let net4 = hub.new_network();

        // net1 and net2 already connected
        net1.dial_mock(&net2);

        // event stream starts here
        let mut events = net1.subscribe_events();
        let peer_ids = net1.get_peers();

        // net1 dials net3
        net1.dial_mock(&net3);

        // net1 is dialed by net3
        net4.dial_mock(&net1);

        // net1 and net2 already connected
        assert_eq!(peer_ids.len(), 1);
        assert_eq!(peer_ids[0], net2.peer_id());

        assert_peer_joined(&mut events, net3.peer_id()).await;
        assert_peer_joined(&mut events, net4.peer_id()).await;

        // test get_peers
        let mut peer_ids = net1.get_peers();
        peer_ids.sort();
        let mut expected_peer_ids = vec![net2.peer_id(), net3.peer_id(), net4.peer_id()];
        expected_peer_ids.sort();

        assert_eq!(peer_ids, expected_peer_ids);
    }

    // Copied straight from nimiq_network_libp2p::network

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
    pub struct TestRecord {
        x: i32,
    }

    impl TaggedSignable for TestRecord {
        const TAG: u8 = 0x22;
    }

    #[test(tokio::test)]
    async fn dht_put_and_get() {
        let mut hub = MockHub::new();
        let mut rng = test_rng(false);
        let net1 = hub.new_network();
        let net2 = hub.new_network();
        net1.dial_mock(&net2);

        let put_record = TestRecord { x: 420 };
        let keypair = KeyPair::generate(&mut rng);

        net1.dht_put(b"foo", &put_record, &keypair).await.unwrap();

        let fetched_record = net2
            .dht_get::<_, TestRecord, KeyPair>(b"foo")
            .await
            .unwrap();

        assert_eq!(fetched_record, Some(put_record));
    }

    pub struct TestTopic;

    impl Topic for TestTopic {
        type Item = TestRecord;

        const BUFFER_SIZE: usize = 8;
        const NAME: &'static str = "test-wasm";
        const VALIDATE: bool = false;
    }

    fn consume_stream<T: std::fmt::Debug>(
        mut stream: impl Stream<Item = T> + Unpin + Send + 'static,
    ) {
        spawn(async move { while stream.next().await.is_some() {} });
    }

    #[test(tokio::test)]
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
            Err(MockNetworkError::AlreadyUnsubscribed(
                TestTopic::NAME.to_string()
            )),
            net1.unsubscribe::<TestTopic>().await
        );
    }
}
