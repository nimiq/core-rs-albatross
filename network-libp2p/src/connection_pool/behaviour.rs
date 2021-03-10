use std::{
    collections::HashSet,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::StreamExt;
use libp2p::{
    core::connection::ConnectionId,
    swarm::{
        IntoProtocolsHandler, NetworkBehaviour, NetworkBehaviourAction, PollParameters,
        ProtocolsHandler,
    },
    Multiaddr, PeerId,
};

use parking_lot::RwLock;
use wasm_timer::{Instant, Interval};

use crate::discovery::peer_contacts::{PeerContactBook, PeerContactInfo};

use super::handler::ConnectionPoolHandler;

#[derive(Clone, Debug)]
pub enum ConnectionPoolEvent {}

pub struct ConnectionPoolBehaviour {
    peer_contact_book: Arc<RwLock<PeerContactBook>>,
    connected_peers: HashSet<PeerId>,
    pending_connections: Vec<Arc<PeerContactInfo>>,
    next_check_timeout: Option<Interval>,
}

impl ConnectionPoolBehaviour {
    pub fn new(peer_contact_book: Arc<RwLock<PeerContactBook>>) -> Self {
        let mut pb = Self {
            peer_contact_book,
            connected_peers: HashSet::new(),
            pending_connections: vec![],
            next_check_timeout: None,
        };
        pb.start_connecting();
        pb
    }

    fn start_connecting(&mut self) {
        if self.next_check_timeout.is_none() {
            self.next_check_timeout =
                Some(Interval::new_at(Instant::now(), Duration::from_secs(3)));
        }
    }

    fn maintain_peers(&mut self) {
        log::trace!(
            "Maintaining peers; #peers: {} peers: {:?}",
            &self.connected_peers.len(),
            &self.connected_peers
        );
        if self.pending_connections.len() == 0 {
            self.pending_connections = self
                .peer_contact_book
                .read()
                .get_next_connections(4 - self.connected_peers.len(), &self.connected_peers);
        }
    }
}

// impl NetworkBehaviourEventProcess<NetworkEvent<Peer>> for PeersBehaviour {
//     fn inject_event(&mut self, event: NetworkEvent<Peer>) {
//         log::error!(">>>>>> PeersBehaviour::inject_event: {:?}", event);

//         match event {
//             NetworkEvent::PeerJoined(peer) => {
//                 self.connected_peers.insert(peer.id)
//             },
//             NetworkEvent::PeerLeft(peer) => {
//                 self.connected_peers.remove(&peer.id)
//             },
//         };

//         self.emit_event(event);
//     }
// }

impl NetworkBehaviour for ConnectionPoolBehaviour {
    type ProtocolsHandler = ConnectionPoolHandler;
    type OutEvent = ConnectionPoolEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        ConnectionPoolHandler::new()
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<Multiaddr> {
        self.peer_contact_book
            .read()
            .get(peer_id)
            .map(|e| e.contact().addresses.clone())
            .or(Some(vec![]))
            .unwrap()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        if self.connected_peers.insert(peer_id.clone()) {
            log::trace!("{:?} added to connected set of peers", peer_id);
        } else {
            log::debug!("{:?} already part of connected set of peers", peer_id);
        }

        self.maintain_peers();
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        if self.connected_peers.remove(peer_id) {
            log::trace!("{:?} removed from connected set of peers", peer_id);
        } else {
            log::debug!("{:?}was not part of connected set of peers", peer_id);
        }

        self.maintain_peers();
    }

    fn inject_event(
        &mut self,
        _peer_id: PeerId,
        _connection: ConnectionId,
        _event: <<Self::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::OutEvent,
    ) {
    }

    fn poll(&mut self, cx: &mut Context<'_>, _params: &mut impl PollParameters) -> Poll<NetworkBehaviourAction<<<Self::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::InEvent, Self::OutEvent>>{
        if self.pending_connections.len() == 0 {
            if let Some(mut timer) = self.next_check_timeout.take() {
                if let Poll::Ready(Some(_)) = timer.poll_next_unpin(cx) {
                    self.maintain_peers();
                }
                self.next_check_timeout = Some(timer);
            }
            // if there is no timer yet we do nothing as that means the network has not yet started.
        } else {
            log::trace!(
                "Creating Dial Action; remaining pending elements: {}",
                self.pending_connections.len() - 1
            );
            return Poll::Ready(NetworkBehaviourAction::DialPeer {
                peer_id: self.pending_connections.pop().unwrap().peer_id().clone(),
                condition: libp2p::swarm::DialPeerCondition::Always,
            });
        }

        Poll::Pending
    }
}
