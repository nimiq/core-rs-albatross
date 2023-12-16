use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use libp2p::{
    core::{
        multiaddr::{multiaddr, Multiaddr},
        transport::MemoryTransport,
        upgrade::Version,
    },
    identity::Keypair,
    noise,
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        Swarm, SwarmEvent,
    },
    yamux, PeerId, SwarmBuilder, Transport,
};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;
use nimiq_network_libp2p::discovery::{
    self,
    peer_contacts::{PeerContact, PeerContactBook, SignedPeerContact},
};
use nimiq_test_log::test;
use parking_lot::RwLock;
use rand::{thread_rng, Rng};

struct TestNode {
    peer_id: PeerId,
    swarm: Swarm<discovery::Behaviour>,
    peer_contact_book: Arc<RwLock<PeerContactBook>>,
    address: Multiaddr,
}

impl TestNode {
    pub fn new() -> Self {
        let keypair = Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());

        let base_transport = MemoryTransport::default();
        let address = multiaddr![Memory(thread_rng().gen::<u64>())];

        log::info!(%peer_id, %address);

        let transport = base_transport
            .upgrade(Version::V1) // `Version::V1Lazy` Allows for 0-RTT negotiation
            .authenticate(noise::Config::new(&keypair).unwrap())
            .multiplex(yamux::Config::default())
            .timeout(Duration::from_secs(20))
            .boxed();

        let config = discovery::Config {
            genesis_hash: Blake2bHash::default(),
            update_interval: Duration::from_secs(10),
            min_send_update_interval: Duration::from_secs(5),
            update_limit: 64,
            required_services: Services::FULL_BLOCKS,
            min_recv_update_interval: Duration::from_secs(1),
            house_keeping_interval: Duration::from_secs(1),
            keep_alive: true,
        };

        let peer_contact = PeerContact {
            addresses: Some(address.clone()).into_iter().collect(),
            public_key: keypair.public(),
            services: config.required_services,
            timestamp: None,
        }
        .sign(&keypair);

        let peer_contact_book = Arc::new(RwLock::new(PeerContactBook::new(peer_contact)));

        let behaviour =
            discovery::Behaviour::new(config, keypair.clone(), Arc::clone(&peer_contact_book));

        let mut swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_other_transport(|_| transport)
            .unwrap()
            .with_behaviour(|_| behaviour)
            .unwrap()
            .build();

        Swarm::listen_on(&mut swarm, address.clone()).unwrap();

        TestNode {
            peer_id,
            swarm,
            peer_contact_book,
            address,
        }
    }

    pub fn dial(&mut self, address: Multiaddr) {
        Swarm::dial(
            &mut self.swarm,
            DialOpts::unknown_peer_id().address(address).build(),
        )
        .unwrap();
    }

    pub fn dial_peer_id(&mut self, peer_id: &PeerId) {
        Swarm::dial(
            &mut self.swarm,
            DialOpts::peer_id(*peer_id)
                .condition(PeerCondition::Disconnected)
                .build(),
        )
        .unwrap();
    }
}

fn random_peer_contact(n: usize, services: Services) -> SignedPeerContact {
    let keypair = Keypair::generate_ed25519();

    let mut peer_contact = PeerContact {
        addresses: vec![format!("/dns/test{}.local/tcp/443/wss", n).parse().unwrap()],
        public_key: keypair.public(),
        services,
        timestamp: None,
    };

    peer_contact.set_current_time();

    peer_contact.sign(&keypair)
}

fn test_peers_in_contact_book(
    peer_contact_book: &PeerContactBook,
    peer_contacts: &[SignedPeerContact],
) {
    for peer_contact in peer_contacts {
        let peer_id = peer_contact.public_key().clone().to_peer_id();
        log::info!(%peer_id, "Checking if peer ID is in peer contact book");
        let peer_contact_in_book = peer_contact_book.get(&peer_id).expect("Peer ID not found");
        assert_eq!(
            peer_contact,
            peer_contact_in_book.signed(),
            "peer contacts differ"
        );
    }
}

#[test(tokio::test)]
pub async fn test_exchanging_peers() {
    // create nodes
    let mut node1 = TestNode::new();
    let node2 = TestNode::new();

    let peer_contact_book1 = Arc::clone(&node1.peer_contact_book);
    let peer_contact_book2 = Arc::clone(&node2.peer_contact_book);

    // known peer contacts of the first node
    let mut node1_peer_contacts = vec![
        random_peer_contact(10, Services::FULL_BLOCKS),
        random_peer_contact(11, Services::FULL_BLOCKS | Services::HISTORY),
    ];

    // known peer contacts of the first node
    let mut node2_peer_contacts = vec![
        random_peer_contact(13, Services::FULL_BLOCKS),
        random_peer_contact(14, Services::FULL_BLOCKS | Services::HISTORY),
        random_peer_contact(15, Services::FULL_BLOCKS | Services::ACCOUNTS_PROOF),
    ];

    // insert peers into node's contact books
    peer_contact_book1
        .write()
        .insert_all(node1_peer_contacts.clone());
    peer_contact_book2
        .write()
        .insert_all(node2_peer_contacts.clone());

    // connect
    node1.dial(node2.address.clone());

    // Run swarm for some time
    let mut t = 0;
    futures::stream::select(node1.swarm, node2.swarm)
        .take_while(move |e| {
            log::info!(event = ?e, "Swarm event");

            if let SwarmEvent::Behaviour(discovery::Event::Update) = e {
                t += 1;
            }

            async move { t < 2 }
        })
        .for_each(|_| async {})
        .await;

    let mut all_peer_contacts = vec![];
    all_peer_contacts.append(&mut node1_peer_contacts);
    all_peer_contacts.append(&mut node2_peer_contacts);

    log::info!("Checking peer 1 contact book");
    test_peers_in_contact_book(&peer_contact_book1.read(), &all_peer_contacts);
    log::info!("Checking peer 2 contact book");
    test_peers_in_contact_book(&peer_contact_book2.read(), &all_peer_contacts);
}

#[test(tokio::test)]
pub async fn test_dialing_peer_from_contacts() {
    // create nodes
    let mut node1 = TestNode::new();
    let node2 = TestNode::new();

    let peer_contact_book1 = Arc::clone(&node1.peer_contact_book);
    let peer_contact_book2 = Arc::clone(&node2.peer_contact_book);

    let peer2_contact = peer_contact_book2.read().get_own_contact().signed().clone();
    let peer2_id = node2.peer_id;

    // insert peer address of node 2 into node 1's address book
    peer_contact_book1.write().insert(peer2_contact);

    // Dial node 2 from node 1 using only peer ID.
    node1.dial_peer_id(&peer2_id);

    // Just run node 2
    tokio::spawn(async move {
        node2.swarm.for_each(|_| async {}).await;
    });

    if let Some(SwarmEvent::Behaviour(discovery::Event::Established {
        peer_id,
        peer_address: _,
        peer_contact: _,
    })) = node1.swarm.next().await
    {
        log::info!(%peer_id, "Established PEX with peer");
        assert_eq!(peer2_id, peer_id);
    }
}

#[test]
fn test_housekeeping() {
    let mut peer_contact_book = PeerContactBook::new(random_peer_contact(1, Services::FULL_BLOCKS));

    let fresh_contact = random_peer_contact(1, Services::FULL_BLOCKS);

    let old_contact = {
        let keypair = Keypair::generate_ed25519();

        let mut peer_contact = PeerContact {
            addresses: vec!["/dns/test_old.local/tcp/443/wss".parse().unwrap()],
            public_key: keypair.public(),
            services: Services::FULL_BLOCKS,
            timestamp: None,
        };

        peer_contact.set_current_time();
        peer_contact
            .timestamp
            .as_mut()
            .map(|t| *t -= PeerContactBook::MAX_PEER_AGE * 2); // twice as older

        peer_contact.sign(&keypair)
    };

    // Insert fresh contact and check that it was inserted
    peer_contact_book.insert(fresh_contact.clone());
    let peer_contact = peer_contact_book
        .get(&fresh_contact.public_key().clone().to_peer_id())
        .unwrap();
    assert_eq!(peer_contact.contact(), &fresh_contact.inner);

    // Insert old contact and check that it was inserted
    peer_contact_book.insert(old_contact.clone());
    let peer_contact = peer_contact_book
        .get(&old_contact.public_key().clone().to_peer_id())
        .unwrap();
    assert_eq!(peer_contact.contact(), &old_contact.inner);

    // Call house-keeping on peer contact book
    peer_contact_book.house_keeping();

    // Check that fresh contact is still in there
    let peer_contact = peer_contact_book
        .get(&fresh_contact.public_key().clone().to_peer_id())
        .unwrap();
    assert_eq!(peer_contact.contact(), &fresh_contact.inner);

    // Check that old contact is not in there
    assert!(peer_contact_book
        .get(&old_contact.public_key().clone().to_peer_id())
        .is_none());
}
