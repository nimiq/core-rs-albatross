use std::collections::VecDeque;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use libp2p::{
    core::{either::EitherError, upgrade::ReadOneError},
    gossipsub::{error::GossipsubHandlerError, Gossipsub, GossipsubEvent, MessageAuthenticity},
    identify::{Identify, IdentifyEvent},
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
    swarm::{NetworkBehaviourAction, NetworkBehaviourEventProcess, PollParameters},
    NetworkBehaviour,
};
use parking_lot::RwLock;

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
    network::Config,
};

pub type NimiqNetworkBehaviourError = EitherError<
    EitherError<
        EitherError<
            EitherError<EitherError<DiscoveryError, ConnectionPoolError>, MessageError>,
            std::io::Error,
        >,
        GossipsubHandlerError,
    >,
    ReadOneError,
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
        let peers = ConnectionPoolBehaviour::new(peer_contact_book);

        let message = MessageBehaviour::new(config.message);

        let store = MemoryStore::new(peer_id);
        let kademlia = Kademlia::with_config(peer_id, store, config.kademlia);
        let gossipsub = Gossipsub::new(
            MessageAuthenticity::Signed(config.keypair),
            config.gossipsub,
        )
        .expect("Wrong configuration");
        let identify = Identify::new(
            "/albatross/2.0".to_string(),
            "albatross_node".to_string(),
            public_key,
        );

        Self {
            discovery,
            message,
            peers,
            kademlia,
            gossipsub,
            identify,
            events: VecDeque::new(),
            waker: None,
        }
    }

    fn poll_event<T>(
        &mut self,
        cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<T, NimiqEvent>> {
        if let Some(event) = self.events.pop_front() {
            log::trace!("NimiqBehaviour: emitting event: {:?}", event);
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
    fn inject_event(&mut self, event: DiscoveryEvent) {
        log::trace!("discovery event: {:?}", event);
    }
}

impl NetworkBehaviourEventProcess<NetworkEvent<Peer>> for NimiqBehaviour {
    fn inject_event(&mut self, event: NetworkEvent<Peer>) {
        log::trace!("NimiqBehaviour::inject_event: {:?}", event);
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        log::trace!("NimiqBehaviour::inject_event: {:?}", event);
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: GossipsubEvent) {
        log::trace!("NimiqBehaviour::inject_event: {:?}", event);
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: IdentifyEvent) {
        log::trace!("NimiqBehaviour::inject_event: {:?}", event);
        self.emit_event(event);
    }
}

impl NetworkBehaviourEventProcess<ConnectionPoolEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: ConnectionPoolEvent) {
        log::trace!("NimiqBehaviour::inject_event: {:?}", event);
        self.emit_event(event);
    }
}
