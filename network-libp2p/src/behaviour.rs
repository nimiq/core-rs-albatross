use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures::channel::mpsc;
use futures::task::{Context, Poll};
use futures::{ready, StreamExt};
use ip_network::IpNetwork;
use libp2p::core::connection::ConnectionId;
use libp2p::core::multiaddr::Protocol;
use libp2p::core::ConnectedPoint;
use libp2p::core::Multiaddr;
use libp2p::swarm::{
    NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters, ProtocolsHandler,
};
use libp2p::PeerId;

use network_interface::network::NetworkEvent;

use crate::handler::{NimiqHandler, NimiqHandlerAction};
use crate::peer::{Peer, PeerAction};

const IPV4_SUBNET: u8 = 24;
const IPV6_SUBNET: u8 = 96;
const MAX_INBOUND_CONNECTION_LIMIT: usize = 100;
const MAX_OUTBOUND_CONNECTION_LIMIT: usize = 2;
const MAX_CONNECTIONS_LIMIT: usize = 4000;
const MAX_CONNECTIONS_PER_IP_LIMIT: usize = 10;

pub struct NimiqBehaviour {
    peers: HashMap<PeerId, Arc<Peer>>,
    ip_ban: HashSet<IpNetwork>,
    ip_count: HashMap<IpNetwork, usize>,
    ipv4_count: usize,
    ipv6_count: usize,
    events: VecDeque<NetworkEvent<Peer>>,
    peer_tx: mpsc::Sender<PeerAction>,
    peer_rx: mpsc::Receiver<PeerAction>,
}

impl NimiqBehaviour {
    pub fn new() -> Self {
        let (peer_tx, peer_rx) = mpsc::channel(4096);
        Self {
            peers: HashMap::new(),
            ip_ban: HashSet::new(),
            ip_count: HashMap::new(),
            ipv4_count: 0,
            ipv6_count: 0,
            events: VecDeque::new(),
            peer_tx,
            peer_rx,
        }
    }
}

impl NetworkBehaviour for NimiqBehaviour {
    type ProtocolsHandler = NimiqHandler;
    type OutEvent = NetworkEvent<Peer>;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        NimiqHandler::new()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        Vec::new()
    }

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        conn: &ConnectionId,
        endpoint: &ConnectedPoint,
    ) {
        let mut close_connection = false;

        let (address, subnet_limit) = match endpoint {
            ConnectedPoint::Listener { send_back_addr, .. } => {
                (send_back_addr.clone(), MAX_INBOUND_CONNECTION_LIMIT)
            }
            ConnectedPoint::Dialer { address } => (address.clone(), MAX_OUTBOUND_CONNECTION_LIMIT),
        };

        // Get the IP for this new peer connection
        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => IpNetwork::new(ip, IPV4_SUBNET).unwrap(),
            Some(Protocol::Ip6(ip)) => IpNetwork::new(ip, IPV6_SUBNET).unwrap(),
            _ => return,
        };

        if self.ip_count.get(&ip).is_some() && MAX_CONNECTIONS_PER_IP_LIMIT < *self.ip_count.get(&ip).unwrap() + 1 {
            debug!("Max peer connections per IP limit reached, {}", ip);
            close_connection = true;
        }
        if ip.is_ipv4() && (subnet_limit < self.ipv4_count + 1) {
            debug!("Max peer connections per IPv4 subnet limit reached");
            close_connection = true;
        }
        if ip.is_ipv6() && (subnet_limit < self.ipv6_count + 1) {
            debug!("IPv6 subnet limit reached");
            close_connection = true;
        }
        if MAX_CONNECTIONS_LIMIT < self.peers.keys().len() + 1 {
            debug!("Max peer connections limit reached");
            close_connection = true;
        }
        if self.ip_ban.contains(&ip) {
            debug!("Connection IP is banned, {}", ip);
            close_connection = true;
        }

        if !close_connection {
            // Increment peer counts per IP
            *self.ip_count.entry(ip).or_insert(0) += 1;
            if ip.is_ipv4() {
                self.ipv4_count += 1;
            } else {
                self.ipv6_count += 1;
            }
        }

        // Send the network event to the Swa
        let peer = Arc::new(Peer::new(peer_id.clone(), self.peer_tx.clone()));
        if close_connection {
            self.events.push_back(NetworkEvent::PeerDisconnect(peer));
        } else {
            self.peers
                .insert(peer_id.clone(), Arc::clone(&peer))
                .map(|p| panic!("Duplicate peer {}", p.id));
            self.events.push_back(NetworkEvent::PeerJoined(peer));
        }
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {}

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        let peer = self
            .peers
            .remove(peer_id)
            .expect("Unknown peer disconnected");
        self.events.push_back(NetworkEvent::PeerLeft(peer));
    }

    fn inject_connection_closed(
        &mut self,
        peer: &PeerId,
        conn: &ConnectionId,
        info: &ConnectedPoint,
    ) {
        let address = match info {
            ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr.clone(),
            ConnectedPoint::Dialer { address } => address.clone(),
        };

        // Get the IP for the closing peer connection
        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => IpNetwork::new(ip, IPV4_SUBNET).unwrap(),
            Some(Protocol::Ip6(ip)) => IpNetwork::new(ip, IPV6_SUBNET).unwrap(),
            _ => return,
        };

        // Decrement peer counts per IP
        *self.ip_count.entry(ip).or_insert(1) -= 1;
        if *self.ip_count.get(&ip).unwrap() == 0 {
            self.ip_count.remove(&ip);
        }

        if ip.is_ipv4() {
            self.ipv4_count -= 1;
        } else {
            self.ipv6_count -= 1;
        }
    }

    fn inject_event(
        &mut self,
        peer_id: PeerId,
        _connection: ConnectionId,
        msg: <Self::ProtocolsHandler as ProtocolsHandler>::OutEvent,
    ) {
        let peer = self
            .peers
            .get(&peer_id)
            .expect("Message received from unknown peer");
        peer.dispatch_inbound_msg(msg);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<NimiqHandlerAction, NetworkEvent<Peer>>> {
        // Emit custom events.
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
        }

        // Notify handlers for outbound messages.
        match ready!(self.peer_rx.poll_next_unpin(cx)) {
            Some(PeerAction::Message(peer_id, msg)) => {
                Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::Any,
                    event: NimiqHandlerAction::Message(msg),
                })
            }
            Some(PeerAction::Close(peer_id)) => {
                Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::All,
                    event: NimiqHandlerAction::Close,
                })
            }
            None => Poll::Pending,
        }
    }
}
