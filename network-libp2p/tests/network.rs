use std::{sync::Arc, time::Duration};

use futures::{Stream, StreamExt};
use libp2p::{
    gossipsub::GossipsubConfigBuilder,
    identity::Keypair,
    multiaddr::{multiaddr, Multiaddr},
    swarm::KeepAlive,
    PeerId,
};
use rand::{thread_rng, Rng};

use beserial::{Deserialize, Serialize};
use beserial_derive::{Deserialize, Serialize};
use nimiq_network_interface::network::{MsgAcceptance, NetworkEvent, Topic};
use nimiq_network_interface::{
    message::{Message, MessageTypeId},
    network::Network as NetworkInterface,
    peer::CloseReason,
};
use nimiq_network_libp2p::{
    discovery::{
        behaviour::DiscoveryConfig,
        peer_contacts::{PeerContact, Protocols, Services},
    },
    Config, Network,
};
use nimiq_test_log::test;
use nimiq_utils::time::OffsetTime;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TestMessage {
    id: u32,
}

impl Message for TestMessage {
    const TYPE_ID: MessageTypeId = MessageTypeId::TestMessage;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TestMessage2 {
    #[beserial(len_type(u8))]
    x: String,
}

impl Message for TestMessage2 {
    const TYPE_ID: MessageTypeId = MessageTypeId::TestMessage2;
}

fn network_config(address: Multiaddr) -> Config {
    let keypair = Keypair::generate_ed25519();

    let mut peer_contact = PeerContact {
        addresses: vec![address],
        public_key: keypair.public(),
        services: Services::all(),
        timestamp: None,
    };
    peer_contact.set_current_time();

    let gossipsub = GossipsubConfigBuilder::default()
        .validation_mode(libp2p::gossipsub::ValidationMode::Permissive)
        .build()
        .expect("Invalid Gossipsub config");

    Config {
        keypair,
        peer_contact,
        seeds: Vec::new(),
        discovery: DiscoveryConfig {
            genesis_hash: Default::default(),
            update_interval: Duration::from_secs(60),
            min_recv_update_interval: Duration::from_secs(30),
            update_limit: 64,
            protocols_filter: Protocols::all(),
            services_filter: Services::all(),
            min_send_update_interval: Duration::from_secs(30),
            house_keeping_interval: Duration::from_secs(60),
            keep_alive: KeepAlive::No,
        },
        kademlia: Default::default(),
        gossipsub,
        memory_transport: true,
    }
}

fn assert_peer_joined(event: &NetworkEvent<PeerId>, wanted_peer_id: &PeerId) {
    if let NetworkEvent::PeerJoined(peer_id) = event {
        assert_eq!(peer_id, wanted_peer_id);
    } else {
        panic!("Event is not a NetworkEvent::PeerJoined: {:?}", event);
    }
}

fn assert_peer_left(event: &NetworkEvent<PeerId>, wanted_peer_id: &PeerId) {
    if let NetworkEvent::PeerLeft(peer_id) = event {
        assert_eq!(peer_id, wanted_peer_id);
    } else {
        panic!("Event is not a NetworkEvent::PeerLeft: {:?}", event);
    }
}

#[derive(Clone, Debug)]
struct TestNetwork {
    next_address: u64,
    addresses: Vec<Multiaddr>,
}

impl TestNetwork {
    pub fn new() -> Self {
        Self {
            next_address: thread_rng().gen::<u64>(),
            addresses: vec![],
        }
    }

    pub async fn spawn(&mut self) -> Network {
        let address = multiaddr![Memory(self.next_address)];
        self.next_address += 1;

        let clock = Arc::new(OffsetTime::new());
        let net = Network::new(clock, network_config(address.clone())).await;
        net.listen_on(vec![address.clone()]).await;

        log::debug!(address = ?address, peer_id = ?net.get_local_peer_id(), "creating node");

        if let Some(dial_address) = self.addresses.first() {
            let mut events = net.subscribe_events();

            log::debug!(address = ?dial_address, "dialing peer");
            net.dial_address(dial_address.clone()).await.unwrap();

            log::debug!("waiting for join event");
            let event = events.next().await;
            log::trace!(event = ?event);
        }

        self.addresses.push(address);

        net
    }
}

async fn create_connected_networks() -> (Network, Network) {
    log::debug!("creating connected test networks:");
    let addr1 = multiaddr![Memory(thread_rng().gen::<u64>())];
    let addr2 = multiaddr![Memory(thread_rng().gen::<u64>())];

    let net1 = Network::new(Arc::new(OffsetTime::new()), network_config(addr1.clone())).await;
    net1.listen_on(vec![addr1.clone()]).await;

    let net2 = Network::new(Arc::new(OffsetTime::new()), network_config(addr2.clone())).await;
    net2.listen_on(vec![addr2.clone()]).await;

    log::debug!(address = ?addr1, peer_id = ?net1.get_local_peer_id(), "Network 1");
    log::debug!(address = ?addr2, peer_id = ?net2.get_local_peer_id(), "Network 2");

    let mut events1 = net1.subscribe_events();
    let mut events2 = net2.subscribe_events();

    log::debug!("dialing peer 1 from peer 2...");
    net2.dial_address(addr1).await.unwrap();

    log::debug!("waiting for join events");

    let event1 = events1.next().await.unwrap().unwrap();
    log::trace!(event1 = ?event1);
    assert_peer_joined(&event1, &net2.get_local_peer_id());

    let event2 = events2.next().await.unwrap().unwrap();
    log::trace!(event2 = ?event2);
    assert_peer_joined(&event2, &net1.get_local_peer_id());

    (net1, net2)
}

async fn create_double_connected_networks() -> (Network, Network) {
    log::debug!("creating connected test networks:");
    let addr1 = multiaddr![Memory(thread_rng().gen::<u64>())];
    let addr2 = multiaddr![Memory(thread_rng().gen::<u64>())];

    let net1 = Network::new(Arc::new(OffsetTime::new()), network_config(addr1.clone())).await;
    net1.listen_on(vec![addr1.clone()]).await;

    let net2 = Network::new(Arc::new(OffsetTime::new()), network_config(addr2.clone())).await;
    net2.listen_on(vec![addr2.clone()]).await;

    log::debug!(address = ?addr1, peer_id = ?net1.get_local_peer_id(), "Network 1");
    log::debug!(address = ?addr2, peer_id = ?net2.get_local_peer_id(), "Network 2");

    let mut events1 = net1.subscribe_events();
    let mut events2 = net2.subscribe_events();

    log::debug!("dialing peer 1 from peer 2 and peer 2 from peer 1");
    assert!(futures::try_join!(net2.dial_address(addr1), net1.dial_address(addr2)).is_ok());

    log::debug!("waiting for join events");

    let event1 = events1.next().await.unwrap().unwrap();
    log::trace!(event1 = ?event1);
    assert_peer_joined(&event1, &net2.get_local_peer_id());

    let event2 = events2.next().await.unwrap().unwrap();
    log::trace!(event2 = ?event2);
    assert_peer_joined(&event2, &net1.get_local_peer_id());

    (net1, net2)
}

async fn create_network_with_n_peers(n_peers: usize) -> Vec<Network> {
    let mut networks = Vec::new();
    let mut addresses = Vec::new();
    let mut rng = rand::thread_rng();

    // Create all the networks and addresses
    for peer in 0..n_peers {
        let addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{}/ws", 9000 + peer)
            .parse()
            .unwrap();

        log::debug!("Creating network: {}", peer);

        addresses.push(addr.clone());

        let network = Network::new(Arc::new(OffsetTime::new()), network_config(addr.clone())).await;
        network.listen_on(vec![addr.clone()]).await;

        log::debug!(address = ?addr, peer_id = ?network.get_local_peer_id(), "Network {}",peer);
        networks.push(network);
    }

    // Connect them
    for peer in 1..n_peers {
        // Dial the previous peer
        log::debug!("Dialing peer: {}", peer);
        networks[peer as usize]
            .dial_address(addresses[(peer - 1) as usize].clone())
            .await
            .unwrap();
    }

    let timeout = tokio::time::Duration::from_secs((n_peers * 2).try_into().unwrap());
    tokio::time::sleep(timeout).await;

    // Verify that each network has all the other peers connected
    for peer in 0..n_peers {
        assert_eq!(networks[peer as usize].get_peers().len(), n_peers - 1);
        assert_eq!(
            networks[peer as usize]
                .network_info()
                .await
                .unwrap()
                .num_peers(),
            n_peers - 1
        );
    }

    // Now disconnect and reconnect a random peer from all peers
    for peer in 0..n_peers {
        let network1 = &networks[peer as usize];
        let peer_id1 = network1.local_peer_id();
        let mut events1 = network1.subscribe_events();

        let mut close_peer = rng.gen_range(0..n_peers);
        while peer == close_peer {
            close_peer = rng.gen_range(0..n_peers);
        }
        let network2 = &networks[close_peer as usize];
        let peer_id2 = network2.local_peer_id();
        let mut events2 = network2.subscribe_events();

        // Verify that both networks have all the other peers connected
        assert_eq!(network1.get_peers().len(), n_peers - 1);
        assert_eq!(network2.get_peers().len(), n_peers - 1);
        assert_eq!(
            network1.network_info().await.unwrap().num_peers(),
            n_peers - 1
        );
        assert_eq!(
            network2.network_info().await.unwrap().num_peers(),
            n_peers - 1
        );

        // Disconnect a random peer
        log::debug!("Disconnecting peer {} from peer {}", close_peer, peer);
        assert!(network1.has_peer(*peer_id2));
        network1.disconnect_peer(*peer_id2, CloseReason::Other);

        // Assert the peer has left both networks
        let close_event1 = events1.next().await.unwrap().unwrap();
        assert_peer_left(&close_event1, peer_id2);
        drop(events1);

        let close_event2 = events2.next().await.unwrap().unwrap();
        assert_peer_left(&close_event2, peer_id1);
        drop(events2);

        // Verify that the networks lost a connection
        assert_eq!(network1.get_peers().len(), n_peers - 2);
        assert_eq!(network2.get_peers().len(), n_peers - 2);
        assert_eq!(
            network1.network_info().await.unwrap().num_peers(),
            n_peers - 2
        );
        assert_eq!(
            network2.network_info().await.unwrap().num_peers(),
            n_peers - 2
        );

        // Now reconnect the peer
        events1 = network1.subscribe_events();
        events2 = network2.subscribe_events();
        log::debug!("Reconnecting peer: {}", close_peer);
        network1
            .dial_address(addresses[close_peer as usize].clone())
            .await
            .unwrap();

        // Assert the peer rejoined the network
        let join_event1 = events1.next().await.unwrap().unwrap();
        assert_peer_joined(&join_event1, peer_id2);

        let join_event2 = events2.next().await.unwrap().unwrap();
        assert_peer_joined(&join_event2, peer_id1);

        // Verify all peers are connected again
        assert_eq!(network1.get_peers().len(), n_peers - 1);
        assert_eq!(network2.get_peers().len(), n_peers - 1);
        assert_eq!(
            network1.network_info().await.unwrap().num_peers(),
            n_peers - 1
        );
        assert_eq!(
            network2.network_info().await.unwrap().num_peers(),
            n_peers - 1
        );
    }

    networks
}

#[test(tokio::test)]
async fn connections_stress_and_reconnect() {
    let peers: usize = 15;
    let networks = create_network_with_n_peers(peers).await;

    assert_eq!(peers, networks.len());
}

#[test(tokio::test)]
async fn two_networks_can_connect() {
    let (net1, net2) = create_connected_networks().await;
    assert_eq!(net1.get_peers().len(), 1);
    assert_eq!(net2.get_peers().len(), 1);

    let peer2 = net1.get_peers()[0];
    let peer1 = net2.get_peers()[0];
    assert_eq!(peer2, net2.get_local_peer_id());
    assert_eq!(peer1, net1.get_local_peer_id());
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 2))]
async fn two_networks_can_connect_double_dial() {
    let (net1, net2) = create_double_connected_networks().await;
    assert_eq!(net1.get_peers().len(), 1);
    assert_eq!(net2.get_peers().len(), 1);

    let peer2 = net1.get_peer(*net2.local_peer_id()).unwrap();
    let peer1 = net2.get_peer(*net1.local_peer_id()).unwrap();
    assert_eq!(peer2.id(), net2.get_local_peer_id());
    assert_eq!(peer1.id(), net1.get_local_peer_id());
}

#[test(tokio::test)]
async fn connections_are_properly_closed_events() {
    let (net1, net2) = create_connected_networks().await;

    assert!(net2.has_peer(*net1.local_peer_id()));

    let mut events1 = net1.subscribe_events();
    let mut events2 = net2.subscribe_events();

    net2.disconnect_peer(*net1.local_peer_id(), CloseReason::Other);
    log::debug!("closed peer");

    let event1 = events1.next().await.unwrap().unwrap();
    assert_peer_left(&event1, net2.local_peer_id());
    log::trace!(event1 = ?event1);

    let event2 = events2.next().await.unwrap().unwrap();
    assert_peer_left(&event2, net1.local_peer_id());
    log::trace!(event2 = ?event2);
}

#[test(tokio::test)]
async fn connections_are_properly_closed_peers() {
    let (net1, net2) = create_connected_networks().await;

    assert!(net2.has_peer(*net1.local_peer_id()));

    let mut events2 = net2.subscribe_events();

    let net1_peer_id = *net1.local_peer_id();
    drop(net1);

    net2.disconnect_peer(net1_peer_id, CloseReason::Other);
    log::debug!("closed peer");

    let event2 = events2.next().await.unwrap().unwrap();
    assert_peer_left(&event2, &net1_peer_id);
    log::trace!(event2 = ?event2);

    assert_eq!(net2.get_peers(), &[]);
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct TestRecord {
    x: i32,
}

#[test(tokio::test)]
async fn dht_put_and_get() {
    let (net1, net2) = create_connected_networks().await;

    // FIXME: Add delay while networks share their addresses
    tokio::time::sleep(Duration::from_secs(2)).await;

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
    const VALIDATE: bool = true;
}

fn consume_stream<T: std::fmt::Debug>(mut stream: impl Stream<Item = T> + Unpin + Send + 'static) {
    tokio::spawn(async move { while stream.next().await.is_some() {} });
}

#[test(tokio::test)]
async fn test_gossipsub() {
    let mut net = TestNetwork::new();

    let net1 = net.spawn().await;
    let net2 = net.spawn().await;

    // Our Gossipsub configuration requires a minimum of 6 peers for the mesh network
    for _ in 0..5i32 {
        let net_n = net.spawn().await;
        let stream_n = net_n.subscribe::<TestTopic>().await.unwrap();
        consume_stream(stream_n);
    }

    let test_message = TestRecord { x: 42 };

    let mut messages = net1.subscribe::<TestTopic>().await.unwrap();
    consume_stream(net2.subscribe::<TestTopic>().await.unwrap());

    tokio::time::sleep(Duration::from_secs(10)).await;

    net2.publish::<TestTopic>(test_message.clone())
        .await
        .unwrap();

    log::info!("Waiting for Gossipsub message...");
    let (received_message, message_id) = messages.next().await.unwrap();
    log::info!("Received Gossipsub message: {:?}", received_message);

    assert_eq!(received_message, test_message);

    // Make sure messages are validated before they are pruned from the memcache
    net1.validate_message::<TestTopic>(message_id, MsgAcceptance::Accept);

    // Call the network_info async function after filling up a topic message buffer to verify that the
    // network drops messages without stalling it's functionality.
    for i in 0..10i32 {
        let msg = TestRecord { x: i };
        net2.publish::<TestTopic>(msg.clone()).await.unwrap();
    }
    net1.network_info().await.unwrap();
}
