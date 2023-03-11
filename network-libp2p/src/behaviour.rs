use std::iter;
use std::sync::Arc;

use libp2p::{
    core::either::EitherError,
    gossipsub::{
        error::GossipsubHandlerError, Gossipsub, GossipsubEvent, MessageAuthenticity,
        PeerScoreParams, PeerScoreThresholds,
    },
    identify::{Behaviour as IdentifyBehaviour, Config as IdentifyConfig, Event as IdentifyEvent},
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
    ping::{
        Behaviour as PingBehaviour, Config as PingConfig, Event as PingEvent,
        Failure as PingFailure,
    },
    request_response::{
        ProtocolSupport, RequestResponse, RequestResponseConfig,
        RequestResponseEvent as ReqResEvent,
    },
    swarm::{ConnectionHandlerUpgrErr, NetworkBehaviour},
    Multiaddr, PeerId,
};
use parking_lot::RwLock;

use nimiq_utils::time::OffsetTime;

use crate::{
    connection_pool::{
        behaviour::{ConnectionPoolBehaviour, ConnectionPoolEvent},
        handler::ConnectionPoolHandlerError,
    },
    discovery::{
        behaviour::{DiscoveryBehaviour, DiscoveryEvent},
        handler::DiscoveryHandlerError,
        peer_contacts::PeerContactBook,
    },
    dispatch::codecs::typed::{IncomingRequest, MessageCodec, OutgoingResponse, ReqResProtocol},
    Config,
};

pub type NimiqNetworkBehaviourError = EitherError<
    EitherError<
        EitherError<
            EitherError<
                EitherError<
                    EitherError<std::io::Error, DiscoveryHandlerError>,
                    GossipsubHandlerError,
                >,
                std::io::Error,
            >,
            PingFailure,
        >,
        ConnectionPoolHandlerError,
    >,
    ConnectionHandlerUpgrErr<std::io::Error>,
>;

pub type RequestResponseEvent = ReqResEvent<IncomingRequest, OutgoingResponse>;

#[derive(Debug)]
pub enum NimiqEvent {
    Dht(KademliaEvent),
    Discovery(DiscoveryEvent),
    Gossip(GossipsubEvent),
    Identify(IdentifyEvent),
    Ping(PingEvent),
    Pool(ConnectionPoolEvent),
    RequestResponse(RequestResponseEvent),
}

impl From<KademliaEvent> for NimiqEvent {
    fn from(event: KademliaEvent) -> Self {
        Self::Dht(event)
    }
}

impl From<DiscoveryEvent> for NimiqEvent {
    fn from(event: DiscoveryEvent) -> Self {
        Self::Discovery(event)
    }
}

impl From<GossipsubEvent> for NimiqEvent {
    fn from(event: GossipsubEvent) -> Self {
        Self::Gossip(event)
    }
}

impl From<IdentifyEvent> for NimiqEvent {
    fn from(event: IdentifyEvent) -> Self {
        Self::Identify(event)
    }
}

impl From<ConnectionPoolEvent> for NimiqEvent {
    fn from(event: ConnectionPoolEvent) -> Self {
        Self::Pool(event)
    }
}

impl From<PingEvent> for NimiqEvent {
    fn from(event: PingEvent) -> Self {
        Self::Ping(event)
    }
}

impl From<RequestResponseEvent> for NimiqEvent {
    fn from(event: RequestResponseEvent) -> Self {
        Self::RequestResponse(event)
    }
}

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "NimiqEvent")]
pub struct NimiqBehaviour {
    pub dht: Kademlia<MemoryStore>,
    pub discovery: DiscoveryBehaviour,
    pub gossipsub: Gossipsub,
    pub identify: IdentifyBehaviour,
    pub ping: PingBehaviour,
    pub pool: ConnectionPoolBehaviour,
    pub request_response: RequestResponse<MessageCodec>,
}

impl NimiqBehaviour {
    pub fn new(
        config: Config,
        clock: Arc<OffsetTime>,
        contacts: Arc<RwLock<PeerContactBook>>,
        peer_score_params: PeerScoreParams,
    ) -> Self {
        let public_key = config.keypair.public();
        let peer_id = public_key.to_peer_id();

        // DHT behaviour
        let store = MemoryStore::new(peer_id);
        let dht = Kademlia::with_config(peer_id, store, config.kademlia);

        // Discovery behaviour
        let discovery = DiscoveryBehaviour::new(
            config.discovery.clone(),
            config.keypair.clone(),
            Arc::clone(&contacts),
            clock,
        );

        // Gossipsub behaviour
        let thresholds = PeerScoreThresholds::default();
        let mut gossipsub = Gossipsub::new(MessageAuthenticity::Author(peer_id), config.gossipsub)
            .expect("Wrong configuration");
        gossipsub
            .with_peer_score(peer_score_params, thresholds)
            .expect("Valid score params and thresholds");

        // Identify behaviour
        let identify_config = IdentifyConfig::new("/albatross/2.0".to_string(), public_key);
        let identify = IdentifyBehaviour::new(identify_config);

        // Ping behaviour:
        // - Send a ping every 15 seconds and timeout at 20 seconds.
        // - The ping behaviour will close the connection if a ping timeouts.
        let ping = PingBehaviour::new(PingConfig::new());

        // Connection pool behaviour
        let pool = ConnectionPoolBehaviour::new(
            Arc::clone(&contacts),
            peer_id,
            config.seeds,
            config.discovery.required_services,
        );

        // Request Response behaviour
        let codec = MessageCodec::default();
        let protocol = ReqResProtocol::Version1;
        let config = RequestResponseConfig::default();
        let request_response =
            RequestResponse::new(codec, iter::once((protocol, ProtocolSupport::Full)), config);

        Self {
            dht,
            discovery,
            gossipsub,
            identify,
            ping,
            pool,
            request_response,
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
