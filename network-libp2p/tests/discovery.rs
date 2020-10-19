use std::{
    time::Duration,
    sync::Arc,
    collections::HashSet,
};

use libp2p::{
    identity::Keypair,
    core::{
        transport::MemoryTransport,
        multiaddr::{Multiaddr, Protocol},
        upgrade::Version,
        muxing::StreamMuxerBox,
    },
    secio::SecioConfig,
    swarm::{Swarm, NetworkBehaviour},
    yamux, PeerId, Transport
};
use futures::{
    future::Either,
    select, Stream, StreamExt
};
use parking_lot::RwLock;

use nimiq_network_libp2p::discovery::{
    behaviour::{Discovery, DiscoveryConfig, DiscoveryEvent},
    peer_contacts::{PeerContact, Services, Protocols},
};
use nimiq_hash::Blake2bHash;
use nimiq_network_libp2p::discovery::peer_contacts::{PeerContactBook, SignedPeerContact};


pub fn memory_multiaddr(a: u64) -> Multiaddr {
    let mut multiaddr = Multiaddr::empty();
    multiaddr.push(Protocol::Memory(a));
    multiaddr
}


struct TestNode {
    keypair: Keypair,
    peer_id: PeerId,
    swarm: Swarm<Discovery>,
    peer_contact_book: Arc<RwLock<PeerContactBook>>,
}

impl TestNode {
    pub fn new(address: u64) -> Self {
        let keypair = Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());

        log::info!("Peer {} ID: {}", address, peer_id);

        let base_transport = MemoryTransport::default();
        let address = memory_multiaddr(address);

        let transport = base_transport
            .upgrade(Version::V1)
            //.upgrade(Version::V1Lazy) // Allows for 0-RTT negotiation
            .authenticate(SecioConfig::new(keypair.clone()))
            .multiplex(yamux::Config::default())
            .map(|(peer, muxer), _| (peer, StreamMuxerBox::new(muxer)))
            .timeout(Duration::from_secs(20));

        let config = DiscoveryConfig {
            genesis_hash: Blake2bHash::default(),
            update_interval: Duration::from_secs(1),
            update_limit: Some(64),
            protocols_filter: Protocols::all(),
            services_filter: Services::all(),
            min_update_interval: Duration::from_secs(1),
        };

        let peer_contact = PeerContact {
            addresses: Some(address.clone()).into_iter().collect(),
            public_key: keypair.public().clone(),
            services: Services::FULL_BLOCKS,
            timestamp: None,
        }.sign(&keypair).unwrap();

        let peer_contact_book = Arc::new(RwLock::new(PeerContactBook::new(peer_contact)));

        let behaviour = Discovery::new(config, keypair.clone(), Arc::clone(&peer_contact_book));

        let mut swarm = Swarm::new(transport, behaviour, peer_id.clone());

        Swarm::listen_on(&mut swarm, address).unwrap();

        TestNode {
            keypair,
            peer_id,
            swarm,
            peer_contact_book,
        }
    }

    pub fn dial(&mut self, address: u64) {
        Swarm::dial_addr(&mut self.swarm, memory_multiaddr(address)).unwrap();
    }
}


fn random_peer_contact(n: usize, services: Services) -> SignedPeerContact {
    let keypair = Keypair::generate_ed25519();

    let mut peer_contact = PeerContact {
        addresses: vec![format!("/dns/test{}.local/tcp/443/wss", n).parse().unwrap()].into_iter().collect(),
        public_key: keypair.public().clone(),
        services,
        timestamp: None,
    };

    peer_contact.set_current_time();

    peer_contact.sign(&keypair).unwrap()
}


#[tokio::test]
pub async fn test_discovery() {
    pretty_env_logger::init();

    // create nodes
    let mut node1 = TestNode::new(1);
    let mut node2 = TestNode::new(2);

    let peer_contact_book1 = Arc::clone(&node1.peer_contact_book);
    let peer_contact_book2 = Arc::clone(&node2.peer_contact_book);

    // known peer contacts of the first node
    let node1_peer_contacts = vec![
        random_peer_contact(10, Services::FULL_BLOCKS),
        random_peer_contact(11, Services::FULL_BLOCKS | Services::BLOCK_HISTORY),
        random_peer_contact(12, Services::BLOCK_PROOF),
    ];

    // known peer contacts of the first node
    let node2_peer_contacts = vec![
        random_peer_contact(13, Services::FULL_BLOCKS),
        random_peer_contact(14, Services::FULL_BLOCKS | Services::BLOCK_HISTORY),
        random_peer_contact(15, Services::CHAIN_PROOF | Services::ACCOUNTS_PROOF),
    ];

    // insert peers into node's contact books
    peer_contact_book1.write().insert_all(node1_peer_contacts);
    peer_contact_book2.write().insert_all(node2_peer_contacts);

    // connect
    node1.dial(2);

    // Run swarm for some time
    futures::stream::select(node1.swarm, node2.swarm)
        .take_while(move |e| {
            println!("Swarm event: {:?}", e);

            let c = if let DiscoveryEvent::Update = e {
                false
            }
            else {
                true
            };

            async move { c }
        })
        .for_each(|_| async {})
        .await;

    // TODO: Check that both peers know all contacts.
}