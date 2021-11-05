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
            EitherError<EitherError<DiscoveryError, ConnectionPoolError>, MessageError>,
            std::io::Error,
        >,
        GossipsubHandlerError,
    >,
    std::io::Error,
>;

#[derive(Debug)]
pub enum NimiqEvent {
    Message(NetworkEvent<Peer>),
    Dht(KademliaEvent),
    Gossip(GossipsubEvent),
    Identify(IdentifyEvent),
    Discovery(DiscoveryEvent),
    Peers(ConnectionPoolEvent),
}

impl From<NetworkEvent<Peer>> for NimiqEvent {
    fn from(event: NetworkEvent<Peer>) -> Self {
        Self::Message(event)
    }
}

impl From<KademliaEvent> for NimiqEvent {
    fn from(event: KademliaEvent) -> Self {
        Self::Dht(event)
    }
}

impl From<ConnectionPoolEvent> for NimiqEvent {
    fn from(event: ConnectionPoolEvent) -> Self {
        Self::Peers(event)
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

impl From<DiscoveryEvent> for NimiqEvent {
    fn from(event: DiscoveryEvent) -> Self {
        Self::Discovery(event)
    }
}

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "NimiqEvent", poll_method = "poll_event")]
pub struct NimiqBehaviour {
    pub discovery: DiscoveryBehaviour,
    pub peers: ConnectionPoolBehaviour,
    pub message: MessageBehaviour,
    //pub limit: LimitBehaviour,
    pub kademlia: Kademlia<MemoryStore>,
    pub gossipsub: Gossipsub,
    pub identify: Identify,

    #[behaviour(ignore)]
    peer_contact_book: Arc<RwLock<PeerContactBook>>,

    #[behaviour(ignore)]
    update_scores: Interval,

    #[behaviour(ignore)]
    events: VecDeque<NimiqEvent>,

    #[behaviour(ignore)]
    waker: Option<Waker>,
}

impl NimiqBehaviour {
    pub fn new(config: Config, clock: Arc<OffsetTime>) -> Self {
        let public_key = config.keypair.public();
        let peer_id = public_key.clone().into_peer_id();

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
        let peers = ConnectionPoolBehaviour::new(peer_contact_book.clone(), config.seeds);

        let message = MessageBehaviour::new(config.message);

        let store = MemoryStore::new(peer_id);

        let kademlia = Kademlia::with_config(peer_id, store, config.kademlia);

        let mut gossipsub = Gossipsub::new(MessageAuthenticity::Author(peer_id), config.gossipsub)
            .expect("Wrong configuration");

        let identify_config = IdentifyConfig::new("/albatross/2.0".to_string(), public_key);
        let identify = Identify::new(identify_config);

        let params = PeerScoreParams::default();
        let thresholds = PeerScoreThresholds::default();
        let update_scores = tokio::time::interval(params.decay_interval);

        gossipsub
            .with_peer_score(params, thresholds)
            .expect("Valid score params and thresholds");

        Self {
            discovery,
            message,
            peers,
            kademlia,
            gossipsub,
            identify,
            peer_contact_book,
            update_scores,
            events: VecDeque::new(),
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

impl NetworkBehaviourEventProcess<DiscoveryEvent> for NimiqBehaviour {
    fn inject_event(&mut self, _event: DiscoveryEvent) {
        self.peers.maintain_peers();
    }
}

impl NetworkBehaviourEventProcess<NetworkEvent<Peer>> for NimiqBehaviour {
    fn inject_event(&mut self, event: NetworkEvent<Peer>) {
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        self.emit_event(event);
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
