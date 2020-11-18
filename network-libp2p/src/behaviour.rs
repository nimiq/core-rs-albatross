use std::collections::VecDeque;
use std::sync::Arc;
use std::task::{Poll, Waker, Context};

use libp2p::NetworkBehaviour;
use libp2p::swarm::{NetworkBehaviourEventProcess, NetworkBehaviourAction, PollParameters};
use libp2p::core::either::EitherOutput;

use nimiq_network_interface::network::NetworkEvent;

use crate::limit::behaviour::{LimitBehaviour, LimitEvent};
use crate::message::{
    behaviour::MessageBehaviour,
    peer::Peer,
};
use crate::{
    message::handler::{
        HandlerOutEvent as MessageEvent, HandlerInEvent as MessageAction,
    },
    limit::handler::HandlerInEvent as LimitAction,
};

#[derive(Default, NetworkBehaviour)]
#[behaviour(event_process = false, out_event = "NetworkEvent<Peer>", poll_method = "poll_event")]
pub struct NimiqBehaviour {
    pub message_behaviour: MessageBehaviour,
    pub limit_behaviour: LimitBehaviour,

    #[behaviour(ignore)]
    events: VecDeque<NetworkEvent<Peer>>,

    #[behaviour(ignore)]
    waker: Option<Waker>,
}

impl NimiqBehaviour {
    fn poll_event(&mut self, cx: &mut Context<'_>, _params: &mut impl PollParameters) -> Poll<NetworkBehaviourAction<EitherOutput<MessageAction, LimitAction>, NetworkEvent<Peer>>> {
        if let Some(event) = self.events.pop_front() {
            Poll::Ready(NetworkBehaviourAction::GenerateEvent(event))
        }
        else{
            // Register waker, if we're waiting for an event.
            if self.waker.is_none() {
                self.waker = Some(cx.waker().clone());
            }

            Poll::Pending
        }
    }
}

impl NetworkBehaviourEventProcess<NetworkEvent<Peer>> for NimiqBehaviour {
    fn inject_event(&mut self, event: NetworkEvent<Peer>) {
        log::debug!("event: {:?}", event);
        match event {
            NetworkEvent::PeerJoined(peer) => {
                self.limit_behaviour.peers
                    .insert(peer.id.clone(), Arc::clone(&peer))
                    .map(|p| panic!("Duplicate peer {}", p.id));
                self.events.push_back(NetworkEvent::PeerJoined(peer));
            },
            NetworkEvent::PeerLeft(peer) => {
                self.events.push_back(NetworkEvent::PeerLeft(peer));
            },
        }

        // Wake up any task that is waiting for events
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

impl From<MessageEvent> for NetworkEvent<Peer> {
    fn from(_event: MessageEvent) -> Self {
        unimplemented!();
    }
}

impl From<LimitEvent> for NetworkEvent<Peer> {
    fn from(_event: LimitEvent) -> Self {
        unimplemented!();
    }
}