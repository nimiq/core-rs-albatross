use std::{iter, sync::Arc};

use libp2p::{
    connection_limits, gossipsub, identify,
    kad::{self, store::MemoryStore},
    ping, request_response,
    swarm::NetworkBehaviour,
    Multiaddr, PeerId, StreamProtocol,
};
use parking_lot::RwLock;

use crate::{
    connection_pool,
    discovery::{self, peer_contacts::PeerContactBook},
    dispatch::codecs::MessageCodec,
    Config,
};

/// Maximum simultaneous libp2p connections per peer
const MAX_CONNECTIONS_PER_PEER: u32 = 2;

/// Network behaviour.
/// This is composed of several other behaviours that build a tree of behaviours using
/// the `NetworkBehaviour` macro and the order of listed behaviours matters.
/// The first behaviours are behaviours that can close connections before establishing them
/// such as connection limits and the connection pool. They must be at the top since they
/// other behaviours such as request-response do not handle well that a connection is
/// denied in a behaviour that is "after".
/// See: https://github.com/libp2p/rust-libp2p/pull/4777#discussion_r1389951783.
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub connection_limits: connection_limits::Behaviour,
    pub pool: connection_pool::Behaviour,
    pub discovery: discovery::Behaviour,
    pub dht: kad::Behaviour<MemoryStore>,
    pub gossipsub: gossipsub::Behaviour,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub request_response: request_response::Behaviour<MessageCodec>,
}

impl Behaviour {
    pub fn new(
        config: Config,
        contacts: Arc<RwLock<PeerContactBook>>,
        peer_score_params: gossipsub::PeerScoreParams,
    ) -> Self {
        let public_key = config.keypair.public();
        let peer_id = public_key.to_peer_id();

        // DHT behaviour
        let store = MemoryStore::new(peer_id);
        let mut dht = kad::Behaviour::with_config(peer_id, store, config.kademlia);
        // Fixme: This could be avoided with a protocol such as Autonat that properly set external addresses to the
        // swarm and also avoids us to add addresses that are purely connection candidates.
        dht.set_mode(Some(kad::Mode::Server));

        // Discovery behaviour
        let discovery = discovery::Behaviour::new(
            config.discovery.clone(),
            config.keypair.clone(),
            Arc::clone(&contacts),
        );

        // Gossipsub behaviour
        let thresholds = gossipsub::PeerScoreThresholds::default();
        let mut gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Author(peer_id),
            config.gossipsub,
        )
        .expect("Wrong configuration");
        gossipsub
            .with_peer_score(peer_score_params, thresholds)
            .expect("Valid score params and thresholds");

        // Identify behaviour
        let identify_config = identify::Config::new("/albatross/2.0".to_string(), public_key);
        let identify = identify::Behaviour::new(identify_config);

        // Ping behaviour:
        // - Send a ping every 15 seconds and timeout at 20 seconds.
        // - The ping behaviour will close the connection if a ping timeouts.
        let ping = ping::Behaviour::new(ping::Config::new());

        // Connection pool behaviour
        let pool = connection_pool::Behaviour::new(
            Arc::clone(&contacts),
            peer_id,
            config.seeds,
            config.discovery.required_services,
        );

        // Request Response behaviour
        let protocol = StreamProtocol::new("/nimiq/reqres/0.0.1");
        let config = request_response::Config::default();
        let request_response = request_response::Behaviour::new(
            iter::once((protocol, request_response::ProtocolSupport::Full)),
            config,
        );

        // Connection limits behaviour
        let limits = connection_limits::ConnectionLimits::default()
            .with_max_pending_incoming(Some(16))
            .with_max_pending_outgoing(Some(16))
            .with_max_established_incoming(Some(4800))
            .with_max_established_outgoing(Some(4800))
            .with_max_established_per_peer(Some(MAX_CONNECTIONS_PER_PEER));
        let connection_limits = connection_limits::Behaviour::new(limits);

        Self {
            dht,
            discovery,
            gossipsub,
            identify,
            ping,
            pool,
            request_response,
            connection_limits,
        }
    }

    /// Adds a peer address into the DHT
    pub fn add_peer_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        // Add address to the DHT
        self.dht.add_address(&peer_id, address);
    }

    /// Removes a peer from the DHT
    pub fn remove_peer(&mut self, peer_id: PeerId) {
        self.dht.remove_peer(&peer_id);
    }

    /// Removes a peer address from the DHT
    pub fn remove_peer_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        // Remove address from the DHT
        self.dht.remove_address(&peer_id, &address);
    }

    /// Updates the scores of all peers in the peer contact book.
    /// Updates are performed with the score values of Gossipsub
    pub fn update_scores(&self, contacts: Arc<RwLock<PeerContactBook>>) {
        contacts.read().update_scores(&self.gossipsub);
    }
}
