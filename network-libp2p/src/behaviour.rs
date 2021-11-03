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
    swarm::{NetworkBehaviourAction, NetworkBehaviourEventProcess, PollParameters},
    NetworkBehaviour,
};
use parking_lot::RwLock;
use tokio::time::Interval;

use nimiq_network_interface::network::NetworkEvent;
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
    message::{behaviour::MessageBehaviour, handler::HandlerError as MessageError, peer::Peer},
    Config,
};

pub type NimiqNetworkBehaviourError = EitherError<
    EitherError<
        EitherError<
            EitherError<EitherError<std::io::Error, DiscoveryError>, GossipsubHandlerError>,
            std::io::Error,
        >,
        MessageError,
    >,
    ConnectionPoolError,
>;

#[derive(Debug)]
pub enum NimiqEvent {
    Dht(KademliaEvent),
    Discovery(DiscoveryEvent),
    Gossip(GossipsubEvent),
    Identify(IdentifyEvent),
    Message(NetworkEvent<Peer>),
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

impl From<NetworkEvent<Peer>> for NimiqEvent {
    fn from(event: NetworkEvent<Peer>) -> Self {
        Self::Message(event)
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
    pub message: MessageBehaviour,
    pub pool: ConnectionPoolBehaviour,

    #[behaviour(ignore)]
    events: VecDeque<NimiqEvent>,

    #[behaviour(ignore)]
    peer_contact_book: Arc<RwLock<PeerContactBook>>,

    #[behaviour(ignore)]
    update_scores: Interval,

    #[behaviour(ignore)]
    waker: Option<Waker>,
}

impl NimiqBehaviour {
    pub fn new(config: Config, clock: Arc<OffsetTime>) -> Self {
        let public_key = config.keypair.public();
        let peer_id = public_key.clone().into_peer_id();

        // DHT behaviour
        let store = MemoryStore::new(peer_id);
        let dht = Kademlia::with_config(peer_id, store, config.kademlia);

        // Discovery behaviour
        // TODO: persist to disk
        let peer_contact_book = Arc::new(RwLock::new(PeerContactBook::new(
            Default::default(),
            config.peer_contact.sign(&config.keypair),
        )));
        let discovery = DiscoveryBehaviour::new(
            config.discovery,
            config.keypair.clone(),
            peer_contact_book.clone(),
            clock,
        );

        // Gossipsub behaviour
        let params = PeerScoreParams::default();
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

        // Message behaviour
        let message = MessageBehaviour::new();

        // Connection pool behaviour
        let pool = ConnectionPoolBehaviour::new(peer_contact_book.clone(), config.seeds);

        Self {
            dht,
            discovery,
            gossipsub,
            identify,
            message,
            pool,
            events: VecDeque::new(),
            peer_contact_book,
            update_scores,
            waker: None,
        }
    }

    fn poll_event<T>(
        &mut self,
        cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<T, NimiqEvent>> {
        if self.update_scores.poll_tick(cx).is_ready() {
            self.peer_contact_book.read().update_scores(&self.gossipsub);
        }

        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
        }

        // Register waker, if we're waiting for an event.
        if self.waker.is_none() {
            self.waker = Some(cx.waker().clone());
        }

        Poll::Pending
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

impl NetworkBehaviourEventProcess<NetworkEvent<Peer>> for NimiqBehaviour {
    fn inject_event(&mut self, event: NetworkEvent<Peer>) {
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<ConnectionPoolEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: ConnectionPoolEvent) {
        self.emit_event(event);
    }
}
