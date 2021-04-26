use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::StreamExt;
use ip_network::IpNetwork;
use libp2p::{
    core::connection::ConnectionId,
    core::multiaddr::Protocol,
    core::ConnectedPoint,
    swarm::{
        IntoProtocolsHandler, NetworkBehaviour, NetworkBehaviourAction, PollParameters,
        ProtocolsHandler,
    },
    Multiaddr, PeerId,
};

use parking_lot::RwLock;
use wasm_timer::{Instant, Interval};

use crate::discovery::peer_contacts::{PeerContactBook, PeerContactInfo};

use super::handler::{ConnectionPoolHandler, HandlerInEvent, HandlerOutEvent};

#[derive(Clone, Debug)]
struct ConnectionPoolLimits {
    ip_count: HashMap<IpNetwork, usize>,
    ipv4_count: usize,
    ipv6_count: usize,
}

#[derive(Clone, Debug)]
struct ConnectionPoolLimitsConfig {
    peer_count_max: usize,
    peer_count_per_ip_max: usize,
    outbound_peer_count_per_subnet_max: usize,
    inbound_peer_count_per_subnet_max: usize,
    ipv4_subnet_mask: u8,
    ipv6_subnet_mask: u8,
}

impl Default for ConnectionPoolLimitsConfig {
    fn default() -> Self {
        Self {
            peer_count_max: 4000,
            peer_count_per_ip_max: 20,
            outbound_peer_count_per_subnet_max: 2,
            inbound_peer_count_per_subnet_max: 100,
            ipv4_subnet_mask: 24,
            ipv6_subnet_mask: 96,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ConnectionPoolEvent {
    Disconnect { peer_id: PeerId },
}

pub struct ConnectionPoolBehaviour {
    peer_contact_book: Arc<RwLock<PeerContactBook>>,
    connected_peers: HashSet<PeerId>,
    pending_connections: Vec<Arc<PeerContactInfo>>,
    next_check_timeout: Option<Interval>,
    limits: ConnectionPoolLimits,
    config: ConnectionPoolLimitsConfig,
    banned: HashSet<IpNetwork>,
    pub events: VecDeque<NetworkBehaviourAction<HandlerInEvent, ConnectionPoolEvent>>,
}

impl ConnectionPoolBehaviour {
    pub fn new(peer_contact_book: Arc<RwLock<PeerContactBook>>) -> Self {
        let limits = ConnectionPoolLimits {
            ip_count: HashMap::new(),
            ipv4_count: 0,
            ipv6_count: 0,
        };

        let mut pb = Self {
            peer_contact_book,
            connected_peers: HashSet::new(),
            pending_connections: vec![],
            next_check_timeout: None,
            limits,
            config: ConnectionPoolLimitsConfig::default(),
            banned: HashSet::new(),
            events: VecDeque::new(),
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
        if self.pending_connections.is_empty() {
            self.pending_connections = self
                .peer_contact_book
                .read()
                .get_next_connections(4 - self.connected_peers.len(), &self.connected_peers);
        }
    }
}

// impl NetworkBehaviourEventProcess<NetworkEvent<Peer>> for ConnectionPoolBehaviour {
//     fn inject_event(&mut self, event: NetworkEvent<Peer>) {
//         log::error!(">>>>>> ConnectionPoolBehaviour::inject_event: {:?}", event);

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

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        _conn: &ConnectionId,
        endpoint: &ConnectedPoint,
    ) {
        let address = endpoint.get_remote_address();
        let subnet_limit = match endpoint {
            ConnectedPoint::Dialer { .. } => self.config.outbound_peer_count_per_subnet_max,
            ConnectedPoint::Listener { .. } => self.config.inbound_peer_count_per_subnet_max,
        };

        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => {
                IpNetwork::new_truncate(ip, self.config.ipv4_subnet_mask).unwrap()
            }
            Some(Protocol::Ip6(ip)) => {
                IpNetwork::new_truncate(ip, self.config.ipv6_subnet_mask).unwrap()
            }
            _ => return, // TODO: Review if we need to handle additional protocols
        };

        let mut close_connection = false;

        if self.banned.get(&ip).is_some() {
            debug!("IP is banned, {}", ip);
            close_connection = true;
        }
        if self.limits.ip_count.get(&ip).is_some()
            && self.config.peer_count_per_ip_max < *self.limits.ip_count.get(&ip).unwrap() + 1
        {
            debug!("Max peer connections per IP limit reached, {}", ip);
            close_connection = true;
        }
        if ip.is_ipv4() && (subnet_limit < self.limits.ipv4_count + 1) {
            debug!("Max peer connections per IPv4 subnet limit reached");
            close_connection = true;
        }
        if ip.is_ipv6() && (subnet_limit < self.limits.ipv6_count + 1) {
            debug!("Max peer connections per IPv6 subnet limit reached");
            close_connection = true;
        }
        if self.config.peer_count_max < self.limits.ipv4_count + self.limits.ipv6_count + 1 {
            debug!("Max peer connections limit reached");
            close_connection = true;
        }

        if close_connection {
            self.events.push_back(NetworkBehaviourAction::GenerateEvent(
                ConnectionPoolEvent::Disconnect { peer_id: *peer_id },
            ));
        } else {
            // Increment peer counts per IP
            *self.limits.ip_count.entry(ip).or_insert(0) += 1;
            match ip {
                IpNetwork::V4(..) => self.limits.ipv4_count += 1,
                IpNetwork::V6(..) => self.limits.ipv6_count += 1,
            };
        }
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        if self.connected_peers.insert(peer_id.clone()) {
            log::trace!("{:?} added to connected set of peers", peer_id);
        } else {
            log::debug!("{:?} already part of connected set of peers", peer_id);
        }

        self.maintain_peers();
    }

    fn inject_connection_closed(
        &mut self,
        _peer_id: &PeerId,
        _conn: &ConnectionId,
        info: &ConnectedPoint,
    ) {
        let address = info.get_remote_address();

        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => {
                IpNetwork::new_truncate(ip, self.config.ipv4_subnet_mask).unwrap()
            }
            Some(Protocol::Ip6(ip)) => {
                IpNetwork::new_truncate(ip, self.config.ipv6_subnet_mask).unwrap()
            }
            _ => return, // TODO: Review if we need to handle additional protocols
        };

        // Decrement IP counters
        *self.limits.ip_count.entry(ip).or_insert(1) -= 1;
        if *self.limits.ip_count.get(&ip).unwrap() == 0 {
            self.limits.ip_count.remove(&ip);
        }

        match ip {
            IpNetwork::V4(..) => self.limits.ipv4_count -= 1,
            IpNetwork::V6(..) => self.limits.ipv6_count -= 1,
        };
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        if self.connected_peers.remove(peer_id) {
            log::trace!("{:?} removed from connected set of peers", peer_id);
        } else {
            log::debug!("{:?} was not part of connected set of peers", peer_id);
        }

        self.maintain_peers();
    }

    fn inject_event(
        &mut self,
        _peer_id: PeerId,
        _connection: ConnectionId,
        event: <<Self::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::OutEvent,
    ) {
        match event {
            HandlerOutEvent::Banned { ip } => {
                if self.banned.insert(ip) {
                    log::trace!("{:?} added to banned set of peers", ip);
                } else {
                    log::debug!("{:?} already part of banned set of peers", ip);
                }
            }
            HandlerOutEvent::Unbanned { ip } => {
                if self.banned.remove(&ip) {
                    log::trace!("{:?} removed from banned set of peers", ip);
                } else {
                    log::debug!("{:?} was not part of banned set of peers", ip);
                }
            }
        }
    }

    fn poll(&mut self, cx: &mut Context<'_>, _params: &mut impl PollParameters) -> Poll<NetworkBehaviourAction<<<Self::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::InEvent, Self::OutEvent>>{
        if self.pending_connections.is_empty() {
            if let Some(mut timer) = self.next_check_timeout.take() {
                if let Poll::Ready(Some(_)) = timer.poll_next_unpin(cx) {
                    self.maintain_peers();
                }
                self.next_check_timeout = Some(timer);
            }
            // If there is no timer yet, we do nothing as that means the network has not yet started.
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
