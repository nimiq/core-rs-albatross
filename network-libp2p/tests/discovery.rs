use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};

use futures::StreamExt;
use instant::SystemTime;
use libp2p::{
    core::{
        multiaddr::{multiaddr, Multiaddr},
        transport::{MemoryTransport, PortUse},
        upgrade::{DeniedUpgrade, Version},
        Endpoint,
    },
    identity::Keypair,
    noise,
    swarm::{
        behaviour::FromSwarm,
        dial_opts::{DialOpts, PeerCondition},
        handler::{ConnectionEvent, FullyNegotiatedOutbound},
        ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId,
        NetworkBehaviour, SubstreamProtocol, Swarm, SwarmEvent, ToSwarm,
    },
    yamux, PeerId, SwarmBuilder, Transport,
};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;
use nimiq_network_libp2p::discovery::{
    self,
    peer_contacts::{PeerContact, PeerContactBook, SignedPeerContact},
    protocol::DiscoveryProtocol,
};
use nimiq_test_log::test;
use nimiq_time::timeout;
use nimiq_utils::spawn;
use parking_lot::RwLock;

#[derive(Clone, Debug)]
struct DuplicateDiscoverySubstreamAttackBehaviour {
    negotiated_outbound_streams: Arc<AtomicUsize>,
}

impl DuplicateDiscoverySubstreamAttackBehaviour {
    fn new(negotiated_outbound_streams: Arc<AtomicUsize>) -> Self {
        Self {
            negotiated_outbound_streams,
        }
    }
}

#[derive(Debug)]
struct DuplicateDiscoverySubstreamAttackHandler {
    negotiated_outbound_streams: Arc<AtomicUsize>,
    pending_outbound_requests: u8,
}

impl DuplicateDiscoverySubstreamAttackHandler {
    fn new(negotiated_outbound_streams: Arc<AtomicUsize>, pending_outbound_requests: u8) -> Self {
        Self {
            negotiated_outbound_streams,
            pending_outbound_requests,
        }
    }
}

impl ConnectionHandler for DuplicateDiscoverySubstreamAttackHandler {
    type FromBehaviour = ();
    type ToBehaviour = ();
    type InboundProtocol = DeniedUpgrade;
    type OutboundProtocol = DiscoveryProtocol;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, ()> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<Self::InboundProtocol, Self::OutboundProtocol, (), ()>,
    ) {
        if let ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
            protocol, ..
        }) = event
        {
            self.negotiated_outbound_streams
                .fetch_add(1, Ordering::Relaxed);
            drop(protocol);
        }
    }

    fn on_behaviour_event(&mut self, _event: ()) {}

    fn connection_keep_alive(&self) -> bool {
        true
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<ConnectionHandlerEvent<Self::OutboundProtocol, (), Self::ToBehaviour>> {
        if self.pending_outbound_requests > 0 {
            self.pending_outbound_requests -= 1;
            return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                protocol: SubstreamProtocol::new(DiscoveryProtocol, ()),
            });
        }

        Poll::Pending
    }
}

impl NetworkBehaviour for DuplicateDiscoverySubstreamAttackBehaviour {
    type ConnectionHandler = DuplicateDiscoverySubstreamAttackHandler;
    type ToSwarm = ();

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(DuplicateDiscoverySubstreamAttackHandler::new(
            Arc::clone(&self.negotiated_outbound_streams),
            0,
        ))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(DuplicateDiscoverySubstreamAttackHandler::new(
            Arc::clone(&self.negotiated_outbound_streams),
            2,
        ))
    }

    fn on_swarm_event(&mut self, _event: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        _event: (),
    ) {
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, <Self::ConnectionHandler as ConnectionHandler>::FromBehaviour>>
    {
        Poll::Pending
    }
}

struct TestNode {
    peer_id: PeerId,
    swarm: Swarm<discovery::Behaviour>,
    peer_contact_book: Arc<RwLock<PeerContactBook>>,
    address: Multiaddr,
}

impl TestNode {
    pub fn new() -> Self {
        Self::with_extra_own_address(None)
    }

    /// Creates a node whose own contact also advertises `extra_address`, which `PeerContact::new`
    /// would drop if it isn't supported by all peers.
    pub fn with_extra_own_address(extra_address: Option<Multiaddr>) -> Self {
        let keypair = Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());

        let base_transport = MemoryTransport::default();
        let address = multiaddr![Memory(rand::random::<u64>())];

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
            only_secure_ws_connections: false,
        };

        let mut peer_contact = PeerContact::new(
            Some(address.clone()),
            keypair.public(),
            config.required_services,
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .expect("PeerContact must be creatable");
        peer_contact.addresses.extend(extra_address);
        let peer_contact = peer_contact.sign(&keypair);

        let peer_contact_book = Arc::new(RwLock::new(PeerContactBook::new(
            peer_contact,
            false,
            true,
            true,
        )));

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

    let peer_contact = PeerContact::new(
        Some(
            format!("/dns/test{n}.local/tcp/443/wss")
                .parse()
                .expect("Must be able to parse Mutltiaddr"),
        ),
        keypair.public(),
        services,
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .expect("PeerContact must be creatable");

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
    spawn(async move {
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
    let mut peer_contact_book = PeerContactBook::new(
        random_peer_contact(1, Services::FULL_BLOCKS),
        false,
        true,
        true,
    );

    let fresh_contact = random_peer_contact(1, Services::FULL_BLOCKS);

    let old_contact = {
        let keypair = Keypair::generate_ed25519();

        let peer_contact = PeerContact::new(
            Some("/dns/test_old.local/tcp/443/wss".parse().unwrap()),
            keypair.public(),
            Services::FULL_BLOCKS,
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .saturating_sub(PeerContactBook::MAX_PEER_AGE * 2),
        )
        .expect("Peer contact must be creatable");

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

/// Verify that `exceeds_age` treats future timestamps as exceeded (defense-in-depth).
/// Even if a future-timestamped contact somehow enters the book, housekeeping must evict it.
#[test]
fn test_future_timestamp_contact_evicted_by_housekeeping() {
    use nimiq_network_libp2p::discovery::peer_contacts::PeerContactInfo;

    let future_ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 10 * 365 * 24 * 3600; // ~10 years from now

    let keypair = Keypair::generate_ed25519();
    let peer_contact = PeerContact::new(
        Some("/dns/future.evil.local/tcp/443/wss".parse().unwrap()),
        keypair.public(),
        Services::FULL_BLOCKS,
        future_ts,
    )
    .expect("Peer contact must be creatable");
    let signed = peer_contact.sign(&keypair);
    let info = PeerContactInfo::from(signed);

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();

    assert!(
        info.exceeds_age(Duration::from_secs(PeerContactBook::MAX_PEER_AGE), now),
        "exceeds_age must return true for future-timestamped contacts"
    );
}

/// Verifies that insert_filtered rejects contacts whose timestamp is far in the
/// future, preventing them from entering the peer contact book in the first place.
#[test]
fn test_insert_filtered_rejects_future_timestamp() {
    let own_contact = random_peer_contact(1, Services::FULL_BLOCKS);
    let mut peer_contact_book = PeerContactBook::new(own_contact, false, true, true);

    let future_contact = {
        let keypair = Keypair::generate_ed25519();
        let future_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 10 * 365 * 24 * 3600;

        let peer_contact = PeerContact::new(
            Some("/dns/future.evil.local/tcp/443/wss".parse().unwrap()),
            keypair.public(),
            Services::FULL_BLOCKS,
            future_ts,
        )
        .expect("Peer contact must be creatable");

        peer_contact.sign(&keypair)
    };

    let future_peer_id = future_contact.public_key().clone().to_peer_id();

    peer_contact_book.insert_filtered(future_contact, Services::FULL_BLOCKS, false);

    assert!(
        peer_contact_book.get(&future_peer_id).is_none(),
        "insert_filtered must reject contacts with far-future timestamps, \
         but the contact was accepted — no timestamp validation exists"
    );
}

#[test(tokio::test)]
async fn test_duplicate_discovery_substream_closes_without_panic() {
    let mut victim = TestNode::new();
    let negotiated_outbound_streams = Arc::new(AtomicUsize::new(0));

    let attacker_keypair = Keypair::generate_ed25519();
    let attacker_peer_id = PeerId::from(attacker_keypair.public());

    let transport = MemoryTransport::default()
        .upgrade(Version::V1)
        .authenticate(noise::Config::new(&attacker_keypair).unwrap())
        .multiplex(yamux::Config::default())
        .timeout(Duration::from_secs(20))
        .boxed();

    let mut attacker_swarm = SwarmBuilder::with_existing_identity(attacker_keypair)
        .with_tokio()
        .with_other_transport(|_| transport)
        .unwrap()
        .with_behaviour(|_| {
            DuplicateDiscoverySubstreamAttackBehaviour::new(Arc::clone(
                &negotiated_outbound_streams,
            ))
        })
        .unwrap()
        .build();

    let attacker_address = multiaddr![Memory(rand::random::<u64>())];
    Swarm::listen_on(&mut attacker_swarm, attacker_address).unwrap();

    Swarm::dial(
        &mut attacker_swarm,
        DialOpts::unknown_peer_id()
            .address(victim.address.clone())
            .build(),
    )
    .unwrap();

    let attack = async {
        let mut victim_closed = false;
        let mut attacker_closed = false;

        loop {
            let negotiated = negotiated_outbound_streams.load(Ordering::Relaxed);
            if negotiated >= 2 && victim_closed {
                break;
            }
            assert!(
                !(attacker_closed && negotiated < 2),
                "Attacker connection closed before negotiating duplicate discovery substreams"
            );

            tokio::select! {
                event = victim.swarm.select_next_some() => {
                    if let SwarmEvent::ConnectionClosed { peer_id, .. } = event
                        && peer_id == attacker_peer_id
                    {
                        victim_closed = true;
                    }
                }
                event = attacker_swarm.select_next_some() => {
                    if matches!(event, SwarmEvent::ConnectionClosed { .. }) {
                        attacker_closed = true;
                    }
                }
            }
        }
    };

    if timeout(Duration::from_secs(10), attack).await.is_err() {
        panic!("Timeout while waiting for duplicate discovery attack and victim-side close");
    }

    assert!(
        negotiated_outbound_streams.load(Ordering::Relaxed) >= 2,
        "Attacker failed to negotiate two discovery substreams"
    );
}

#[test]
fn test_insert_filtered_rejects_stale_timestamp() {
    let own_contact = random_peer_contact(1, Services::FULL_BLOCKS);
    let mut peer_contact_book = PeerContactBook::new(own_contact, false, true, true);

    let stale_contact = {
        let keypair = Keypair::generate_ed25519();
        let stale_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(PeerContactBook::MAX_PEER_AGE + 1);

        let peer_contact = PeerContact::new(
            Some("/dns/stale.evil.local/tcp/443/wss".parse().unwrap()),
            keypair.public(),
            Services::FULL_BLOCKS,
            stale_ts,
        )
        .expect("Peer contact must be creatable");

        peer_contact.sign(&keypair)
    };

    let stale_peer_id = stale_contact.public_key().clone().to_peer_id();

    peer_contact_book.insert_filtered(stale_contact, Services::FULL_BLOCKS, false);

    assert!(
        peer_contact_book.get(&stale_peer_id).is_none(),
        "insert_filtered must reject contacts older than MAX_PEER_AGE"
    );
}

#[test]
fn test_insert_filtered_only_replaces_with_newer_timestamp() {
    let own_contact = random_peer_contact(1, Services::FULL_BLOCKS);
    let mut peer_contact_book = PeerContactBook::new(own_contact, false, true, true);

    let keypair = Keypair::generate_ed25519();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let base_ts = now.saturating_sub(60);

    let original_contact = PeerContact::new(
        Some("/dns/peer.old.local/tcp/443/wss".parse().unwrap()),
        keypair.public(),
        Services::FULL_BLOCKS,
        base_ts,
    )
    .expect("Peer contact must be creatable")
    .sign(&keypair);
    let same_timestamp_contact = PeerContact::new(
        Some("/dns/peer.same.local/tcp/443/wss".parse().unwrap()),
        keypair.public(),
        Services::FULL_BLOCKS,
        base_ts,
    )
    .expect("Peer contact must be creatable")
    .sign(&keypair);
    let newer_contact = PeerContact::new(
        Some("/dns/peer.new.local/tcp/443/wss".parse().unwrap()),
        keypair.public(),
        Services::FULL_BLOCKS,
        base_ts + 1,
    )
    .expect("Peer contact must be creatable")
    .sign(&keypair);

    let peer_id = original_contact.public_key().clone().to_peer_id();

    peer_contact_book.insert_filtered(original_contact.clone(), Services::FULL_BLOCKS, false);
    assert_eq!(
        peer_contact_book.get(&peer_id).unwrap().signed(),
        &original_contact,
        "fresh contact should be inserted"
    );

    peer_contact_book.insert_filtered(same_timestamp_contact, Services::FULL_BLOCKS, false);
    assert_eq!(
        peer_contact_book.get(&peer_id).unwrap().signed(),
        &original_contact,
        "same timestamp must not overwrite the existing contact"
    );

    peer_contact_book.insert_filtered(newer_contact.clone(), Services::FULL_BLOCKS, false);
    assert_eq!(
        peer_contact_book.get(&peer_id).unwrap().signed(),
        &newer_contact,
        "newer timestamp must replace the existing contact"
    );
}

#[test]
fn test_insert_rejects_contacts_old_peers_cannot_decode() {
    let own_contact = random_peer_contact(1, Services::FULL_BLOCKS);
    let mut peer_contact_book = PeerContactBook::new(own_contact, false, true, true);

    let keypair = Keypair::generate_ed25519();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Built by hand since `PeerContact::new` drops such addresses from our own contact.
    let mut peer_contact = PeerContact::new(
        Some("/dns/peer.local/tcp/443/wss".parse().unwrap()),
        keypair.public(),
        Services::FULL_BLOCKS,
        now,
    )
    .expect("Peer contact must be creatable");
    peer_contact.addresses.push(
        "/dns/peer.local/tcp/443/wss/http-path/nimiq"
            .parse()
            .unwrap(),
    );
    let contact = peer_contact.sign(&keypair);
    assert!(contact.verify());

    let peer_id = contact.public_key().clone().to_peer_id();

    peer_contact_book.insert(contact.clone());
    peer_contact_book.insert_filtered(contact, Services::FULL_BLOCKS, false);
    assert!(
        peer_contact_book.get(&peer_id).is_none(),
        "contacts with /http-path addresses must not be stored, since they would be relayed"
    );
}

#[test]
fn test_own_contact_drops_addresses_old_peers_cannot_decode() {
    let keypair = Keypair::generate_ed25519();
    let address: Multiaddr = "/dns/peer.local/tcp/443/wss".parse().unwrap();
    let http_path_address: Multiaddr = "/dns/peer.local/tcp/443/wss/http-path/nimiq"
        .parse()
        .unwrap();

    let mut peer_contact = PeerContact::new(
        [address.clone(), http_path_address.clone()],
        keypair.public(),
        Services::FULL_BLOCKS,
        0,
    )
    .expect("Peer contact must be creatable");
    assert_eq!(peer_contact.addresses, vec![address.clone()]);

    peer_contact.add_addresses(vec![http_path_address]);
    assert_eq!(peer_contact.addresses, vec![address]);
}

#[test(tokio::test)]
pub async fn test_peer_with_unsupported_own_address_joins_without_being_relayed() {
    let mut node1 = TestNode::new();
    let node2 = TestNode::with_extra_own_address(Some(
        "/dns/peer.local/tcp/443/wss/http-path/nimiq"
            .parse()
            .unwrap(),
    ));

    let peer_contact_book1 = Arc::clone(&node1.peer_contact_book);
    let peer2_id = node2.peer_id;

    node1.dial(node2.address.clone());

    // Just run node 2
    spawn(async move {
        node2.swarm.for_each(|_| async {}).await;
    });

    let established_peer_id = timeout(Duration::from_secs(10), async {
        while let Some(event) = node1.swarm.next().await {
            if let SwarmEvent::Behaviour(discovery::Event::Established { peer_id, .. }) = event {
                return peer_id;
            }
        }
        panic!("Swarm ended before peer exchange was established");
    })
    .await
    .expect("Peer exchange must be established");

    assert_eq!(established_peer_id, peer2_id);
    assert!(
        peer_contact_book1.read().get(&peer2_id).is_none(),
        "the contact must not be stored, since it would be relayed"
    );
}
