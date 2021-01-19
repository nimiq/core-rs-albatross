use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, SystemTime};

use futures::task::{Context, Poll};
use ip_network::IpNetwork;
use libp2p::core::connection::ConnectionId;
use libp2p::core::multiaddr::Protocol;
use libp2p::core::ConnectedPoint;
use libp2p::core::Multiaddr;
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters, ProtocolsHandler};
use libp2p::PeerId;

use super::handler::{HandlerInEvent, HandlerOutEvent, LimitHandler};

#[derive(Clone, Debug)]
pub struct LimitConfig {
    peer_count_max: usize,
    peer_count_per_ip_max: usize,
    outbound_peer_count_per_subnet_max: usize,
    inbound_peer_count_per_subnet_max: usize,
    ipv4_subnet_mask: u8,
    ipv6_subnet_mask: u8,
    default_ban_time: Duration,
}

impl Default for LimitConfig {
    fn default() -> Self {
        Self {
            peer_count_max: 4000,
            peer_count_per_ip_max: 20,
            outbound_peer_count_per_subnet_max: 2,
            inbound_peer_count_per_subnet_max: 100,
            ipv4_subnet_mask: 24,
            ipv6_subnet_mask: 96,
            default_ban_time: Duration::from_secs(60 * 10), // 10 minutes
        }
    }
}

pub struct LimitBehaviour {
    config: LimitConfig,

    pub peers: HashSet<PeerId>,
    pub ip_ban: HashMap<IpNetwork, SystemTime>,
    ip_count: HashMap<IpNetwork, usize>,
    ipv4_count: usize,
    ipv6_count: usize,
    events: VecDeque<NetworkBehaviourAction<HandlerInEvent, LimitEvent>>,
}

impl Default for LimitBehaviour {
    fn default() -> Self {
        Self::new(LimitConfig::default())
    }
}

#[derive(Clone, Debug)]
pub enum LimitEvent {
    ClosePeers { peers: Vec<PeerId> },
}

impl LimitBehaviour {
    pub fn new(config: LimitConfig) -> Self {
        Self {
            config,
            peers: HashSet::new(),
            ip_ban: HashMap::new(),
            ip_count: HashMap::new(),
            ipv4_count: 0,
            ipv6_count: 0,
            events: VecDeque::new(),
        }
    }
}

impl NetworkBehaviour for LimitBehaviour {
    type ProtocolsHandler = LimitHandler;
    type OutEvent = LimitEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        LimitHandler::default()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        Vec::new()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        log::debug!("LimitBehaviour::inject_connected: {}", peer_id);
        self.peers.insert(peer_id.clone());
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        log::debug!("LimitBehaviour::inject_disconnected: {}", peer_id);
        self.peers.remove(peer_id);
    }

    fn inject_connection_established(&mut self, peer_id: &PeerId, _conn: &ConnectionId, connected_point: &ConnectedPoint) {
        let mut close_connection = false;

        let (address, subnet_limit) = match connected_point {
            ConnectedPoint::Listener { send_back_addr, .. } => (send_back_addr.clone(), self.config.inbound_peer_count_per_subnet_max),
            ConnectedPoint::Dialer { address } => (address.clone(), self.config.outbound_peer_count_per_subnet_max),
        };

        // Get the IP for this new peer connection
        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => IpNetwork::new(ip, self.config.ipv4_subnet_mask).unwrap(),
            Some(Protocol::Ip6(ip)) => IpNetwork::new(ip, self.config.ipv6_subnet_mask).unwrap(),
            _ => return,
        };

        if self.ip_count.get(&ip).is_some() && self.config.peer_count_per_ip_max < *self.ip_count.get(&ip).unwrap() + 1 {
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
        if self.config.peer_count_max < self.ipv4_count + self.ipv6_count + 1 {
            debug!("Max peer connections limit reached");
            close_connection = true;
        }
        if self.ip_ban.get(&ip).is_some() {
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

        } else {
            self.events.push_back(NetworkBehaviourAction::NotifyHandler {
                peer_id: peer_id.clone(),
                handler: NotifyHandler::All,
                event: HandlerInEvent::ClosePeer {
                    peer_id: peer_id.clone(),
                },
            });
        }
    }

    fn inject_connection_closed(&mut self, peer_id: &PeerId, _conn: &ConnectionId, info: &ConnectedPoint) {
        let address = match info {
            ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr.clone(),
            ConnectedPoint::Dialer { address } => address.clone(),
        };

        // Get the IP for the closing peer connection
        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => IpNetwork::new(ip, self.config.ipv4_subnet_mask).unwrap(),
            Some(Protocol::Ip6(ip)) => IpNetwork::new(ip, self.config.ipv6_subnet_mask).unwrap(),
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

    fn inject_event(&mut self, peer_id: PeerId, _connection: ConnectionId, event: <Self::ProtocolsHandler as ProtocolsHandler>::OutEvent) {
        log::trace!("LimitBehaviour::inject_event: peer_id={:?}: {:?}", peer_id, event);

        match event {
            HandlerOutEvent::ClosePeers { peers } => {
                self.events.push_back(NetworkBehaviourAction::GenerateEvent(LimitEvent::ClosePeers {
                    peers,
                }));
            }
        }
    }

    fn poll(&mut self, _cx: &mut Context<'_>, _params: &mut impl PollParameters) -> Poll<NetworkBehaviourAction<HandlerInEvent, LimitEvent>> {
        let ip_ban = self.ip_ban.clone();
        for (ip, time) in ip_ban {
            if time < SystemTime::now() {
                self.ip_ban.remove(&ip);
            }
        }

        // Emit custom events.
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        Poll::Pending
    }
}
