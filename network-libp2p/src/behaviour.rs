use std::collections::VecDeque;
use std::sync::Arc;
use std::task::{Poll, Waker, Context};

use libp2p::NetworkBehaviour;
use libp2p::swarm::{NetworkBehaviourEventProcess, NetworkBehaviourAction, PollParameters};
use libp2p::core::either::EitherOutput;
use parking_lot::RwLock;

use nimiq_network_interface::network::NetworkEvent;

use crate::{
    discovery::{
        behaviour::{DiscoveryBehaviour, DiscoveryEvent},
        handler::{HandlerInEvent as DiscoveryAction},
        peer_contacts::PeerContactBook,
    },
    message::{
        behaviour::MessageBehaviour,
        handler::{HandlerInEvent as MessageAction},
        peer::Peer,
    },
    limit::{
        behaviour::{LimitBehaviour, LimitEvent},
        handler::{HandlerInEvent as LimitAction},
    },
    network::Config,
};


type NimiqNetworkBehaviourAction = NetworkBehaviourAction<
    EitherOutput<
        EitherOutput<
            DiscoveryAction,
            MessageAction,
        >,
        LimitAction,
    >,
    NetworkEvent<Peer>,
>;


#[derive(NetworkBehaviour)]
#[behaviour(out_event = "NetworkEvent<Peer>", poll_method = "poll_event")]
pub struct NimiqBehaviour {
    pub discovery: DiscoveryBehaviour,
    pub message: MessageBehaviour,
    pub limit: LimitBehaviour,

    #[behaviour(ignore)]
    events: VecDeque<NetworkEvent<Peer>>,

    #[behaviour(ignore)]
    waker: Option<Waker>,
}

impl NimiqBehaviour {
    pub fn new(config: Config) -> Self {
        // TODO: persist to disk
        let peer_contact_book = Arc::new(RwLock::new(PeerContactBook::new(
            Default::default(),
            config.peer_contact.sign(&config.keypair)
        )));
        let discovery = DiscoveryBehaviour::new(config.discovery, config.keypair.clone(), peer_contact_book);

        let message = MessageBehaviour::new(config.message);

        let limit = LimitBehaviour::new(config.limit);

        Self {
            discovery,
            message,
            limit,
            events: VecDeque::new(),
            waker: None,
        }
    }

    fn poll_event(&mut self, cx: &mut Context, _params: &mut impl PollParameters) -> Poll<NimiqNetworkBehaviourAction> {
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
        match event {
            NetworkEvent::PeerJoined(peer) => {
                /*self.limit.peers
                    .insert(peer.id.clone(), Arc::clone(&peer))
                    .map(|p| panic!("Duplicate peer {}", p.id));*/

                self.events.push_back(NetworkEvent::PeerJoined(peer));
            },
            NetworkEvent::PeerLeft(peer) => {
                self.events.push_back(NetworkEvent::PeerLeft(peer));
            },
        }

        self.wake();
    }
}

impl NetworkBehaviourEventProcess<LimitEvent> for NimiqBehaviour {
    fn inject_event(&mut self, event: LimitEvent) {
        log::trace!("NimiqBehaviour::inject_event: {:?}", event);
    }
}
