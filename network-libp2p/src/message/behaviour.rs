use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use futures::task::{Context, Poll, Waker};
use libp2p::core::connection::ConnectionId;
use libp2p::core::Multiaddr;
use libp2p::swarm::{
    NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters,
};
use libp2p::{
    core::ConnectedPoint,
    PeerId
};

use nimiq_network_interface::network::NetworkEvent;

use super::{
    peer::Peer,
    handler::{MessageHandler, HandlerInEvent, HandlerOutEvent},
};


#[derive(Clone)]
pub struct MessageConfig {
    // TODO
}

impl Default for MessageConfig {
    fn default() -> Self {
        Self {

        }
    }
}


pub struct MessageBehaviour {
    config: MessageConfig,

    events: VecDeque<NetworkBehaviourAction<HandlerInEvent, NetworkEvent<Peer>>>,

    peers: HashMap<PeerId, Arc<Peer>>,

    waker: Option<Waker>,
}

impl Default for MessageBehaviour {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl MessageBehaviour {
    pub fn new(config: MessageConfig) -> Self {
        Self {
            config,
            peers: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
        }
    }

    pub fn get_peers(&self) -> impl Iterator<Item=&Arc<Peer>> {
        self.peers.values()
    }

    pub fn get_peer(&self, peer_id: &PeerId) -> Option<&Arc<Peer>> {
        self.peers.get(peer_id)
    }

    fn push_event(&mut self, event: NetworkBehaviourAction<HandlerInEvent, NetworkEvent<Peer>>) {
        self.events.push_back(event);
        self.wake();
    }

    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}

impl NetworkBehaviour for MessageBehaviour {
    type ProtocolsHandler = MessageHandler;
    type OutEvent = NetworkEvent<Peer>;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        MessageHandler::new(self.config.clone())
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        vec![]
    }

    fn inject_connected(&mut self, _peer_id: &PeerId) {
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        // No handler exists anymore.
        log::debug!("inject_disconnected: {:?}", peer_id);
    }

    fn inject_connection_established(&mut self, peer_id: &PeerId, connection_id: &ConnectionId, connected_point: &ConnectedPoint) {
        log::info!("Connection established: peer_id={:?}, connection_id={:?}, connected_point={:?}", peer_id, connection_id, connected_point);

        self.events.push_back(NetworkBehaviourAction::NotifyHandler {
            peer_id: peer_id.clone(),
            handler: NotifyHandler::All,
            event: HandlerInEvent::PeerConnected {
                peer_id: peer_id.clone(),
                outbound: connected_point.is_dialer(),
            },
        });
    }

    fn inject_connection_closed(&mut self, peer_id: &PeerId, connection_id: &ConnectionId, connected_point: &ConnectedPoint) {
        log::info!("Connection closed: peer_id={:?}, connection_id={:?}, connected_point={:?}", peer_id, connection_id, connected_point);

        // If we still know this peer, remove it and emit an `PeerLeft` event to the swarm.
        if let Some(peer) = self.peers.remove(peer_id) {
            log::debug!("Peer disconnected: {:?}", peer);
            self.push_event(NetworkBehaviourAction::GenerateEvent(NetworkEvent::PeerLeft(peer)));
        }
    }

    fn inject_event(
        &mut self,
        peer_id: PeerId,
        _connection: ConnectionId,
        event: HandlerOutEvent,
    ) {
        log::debug!("MessageBehaviour::inject_event: peer_id={:?}: {:?}", peer_id, event);
        match event {
            HandlerOutEvent::PeerJoined { peer } => {
                self.peers.insert(peer_id, Arc::clone(&peer));
                self.push_event(NetworkBehaviourAction::GenerateEvent(NetworkEvent::PeerJoined(peer)));
            },
            HandlerOutEvent::PeerClosed { peer, reason } => {
                log::debug!("Peer closed: {:?}, reason={:?}", peer, reason);
                self.peers.remove(&peer_id);
                self.push_event(NetworkBehaviourAction::GenerateEvent(NetworkEvent::PeerLeft(peer)));
            }
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<HandlerInEvent, NetworkEvent<Peer>>> {
        // Emit custom events.
        if let Some(event) = self.events.pop_front() {
            log::debug!("MessageBehaviour::poll: Emitting event: {:?}", event);
            return Poll::Ready(event);
        }

        // Remember the waker
        if self.waker.is_none() {
            self.waker = Some(cx.waker().clone());
        }

        Poll::Pending
    }
}
