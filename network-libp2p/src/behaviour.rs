use std::collections::VecDeque;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use libp2p::{
    core::either::EitherError,
    gossipsub::{
        error::GossipsubHandlerError, Gossipsub, GossipsubEvent, MessageAuthenticity,
        PeerScoreParams, PeerScoreThresholds,
    },
    identify::{Identify, IdentifyConfig, IdentifyEvent},
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
    swarm::{
        NetworkBehaviour, NetworkBehaviourAction, NetworkBehaviourEventProcess, PollParameters,
    },
    Multiaddr, NetworkBehaviour, PeerId,
};
use parking_lot::RwLock;
use tokio::time::Interval;

use nimiq_network_interface::peer_map::ObservablePeerMap;
use nimiq_utils::time::OffsetTime;

use crate::{
    connection_pool::{
        behaviour::{ConnectionPoolBehaviour, ConnectionPoolEvent},
        handler::HandlerError as ConnectionPoolError,
    },
    discovery::{
        behaviour::{DiscoveryBehaviour, DiscoveryEvent},
        handler::HandlerError as DiscoveryError,
        peer_contacts::PeerContactBook,
    },
    peer::Peer,
    Config,
};

pub type NimiqNetworkBehaviourError = EitherError<
    EitherError<
        EitherError<EitherError<std::io::Error, DiscoveryError>, GossipsubHandlerError>,
        std::io::Error,
    >,
    ConnectionPoolError,
>;

#[derive(Debug)]
pub enum NimiqEvent {
    Dht(KademliaEvent),
    Discovery(DiscoveryEvent),
    Gossip(GossipsubEvent),
    Identify(IdentifyEvent),
    Pool(ConnectionPoolEvent),
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

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "NimiqEvent", poll_method = "poll_event")]
pub struct NimiqBehaviour {
    pub dht: Kademlia<MemoryStore>,
    pub discovery: DiscoveryBehaviour,
    pub gossipsub: Gossipsub,
    pub identify: Identify,
    pub pool: ConnectionPoolBehaviour,

    #[behaviour(ignore)]
    contacts: Arc<RwLock<PeerContactBook>>,

    #[behaviour(ignore)]
    events: VecDeque<NimiqEvent>,

    #[behaviour(ignore)]
    update_scores: Interval,

    #[behaviour(ignore)]
    waker: Option<Waker>,
}

impl NimiqBehaviour {
    pub fn new(config: Config, clock: Arc<OffsetTime>, peers: ObservablePeerMap<Peer>) -> Self {
        let public_key = config.keypair.public();
        let peer_id = public_key.clone().to_peer_id();

        // DHT behaviour
        let store = MemoryStore::new(peer_id);
        let dht = Kademlia::with_config(peer_id, store, config.kademlia);

        // Discovery behaviour
        // TODO: persist to disk
        let contacts = Arc::new(RwLock::new(PeerContactBook::new(
            Default::default(),
            config.peer_contact.sign(&config.keypair),
        )));
        let discovery = DiscoveryBehaviour::new(
            config.discovery,
            config.keypair.clone(),
            Arc::clone(&contacts),
            clock,
        );

        // Gossipsub behaviour
        let params = PeerScoreParams {
            ip_colocation_factor_threshold: 20.0,
            ..Default::default()
        };
        let thresholds = PeerScoreThresholds::default();
        let update_scores = tokio::time::interval(params.decay_interval);
        let mut gossipsub = Gossipsub::new(MessageAuthenticity::Author(peer_id), config.gossipsub)
            .expect("Wrong configuration");
        gossipsub
            .with_peer_score(params, thresholds)
            .expect("Valid score params and thresholds");

        // Identify behaviour
        let identify_config = IdentifyConfig::new("/albatross/2.0".to_string(), public_key);
        let identify = Identify::new(identify_config);

        // Connection pool behaviour
        let pool = ConnectionPoolBehaviour::new(Arc::clone(&contacts), config.seeds, peers);

        Self {
            dht,
            discovery,
            gossipsub,
            identify,
            pool,
            events: VecDeque::new(),
            contacts,
            update_scores,
            waker: None,
        }
    }

    fn poll_event(
        &mut self,
        cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<
        NetworkBehaviourAction<
            <Self as NetworkBehaviour>::OutEvent,
            <Self as NetworkBehaviour>::ProtocolsHandler,
        >,
    > {
        if self.update_scores.poll_tick(cx).is_ready() {
            self.contacts.read().update_scores(&self.gossipsub);
        }

        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
        }

        // Register waker, if we're waiting for an event.
        store_waker!(self, waker, cx);

        Poll::Pending
    }

    pub fn add_peer_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        // Add address to the DHT if it's reachable outside of local nodes
        self.dht.add_address(&peer_id, address);
    }

    pub fn remove_peer_address(&mut self, peer_id: PeerId, address: Multiaddr) {
        // Remove address from the DHT
        self.dht.remove_address(&peer_id, &address);
    }

    fn emit_event<E>(&mut self, event: E)
    where
        NimiqEvent: From<E>,
    {
        self.events.push_back(event.into());
        self.wake();
    }

    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<DiscoveryEvent> for NimiqBehaviour {
    fn inject_event(&mut self, _event: DiscoveryEvent) {
        self.pool.maintain_peers();
    }
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: GossipsubEvent) {
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: IdentifyEvent) {
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<ConnectionPoolEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: ConnectionPoolEvent) {
        self.emit_event(event);
    }
}
