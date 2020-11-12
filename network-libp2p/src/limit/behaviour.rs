use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::channel::mpsc;
use futures::task::{Context, Poll};
use ip_network::IpNetwork;
use libp2p::core::connection::ConnectionId;
use libp2p::core::multiaddr::Protocol;
use libp2p::core::ConnectedPoint;
use libp2p::core::Multiaddr;
use libp2p::swarm::{
    NetworkBehaviour, NetworkBehaviourAction, PollParameters, ProtocolsHandler,
};
use libp2p::PeerId;

use nimiq_network_interface::network::NetworkEvent;

use crate::message::peer::Peer;
use super::handler::{
    LimitHandler, HandlerInEvent, HandlerOutEvent,
};


const PEER_COUNT_MAX: usize = 4000;
const PEER_COUNT_PER_IP_MAX: usize = 20;
const OUTBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 2;
const INBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 100;

const IPV4_SUBNET_MASK: u8 = 24;
const IPV6_SUBNET_MASK: u8 = 96;

const DEFAULT_BAN_TIME: Duration = Duration::from_secs(60 * 10);



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

    pub peers: HashMap<PeerId, Arc<Peer>>,
    ip_ban: HashMap<IpNetwork, SystemTime>,
    ip_count: HashMap<IpNetwork, usize>,
    ipv4_count: usize,
    ipv6_count: usize,
    events: VecDeque<LimitEvent>,
}

impl Default for LimitBehaviour {
    fn default() -> Self {
        Self::new(LimitConfig::default())
    }
}


pub enum LimitEvent {

}


impl LimitBehaviour {
    pub fn new(config: LimitConfig) -> Self {
        Self {
            config,
            peers: HashMap::new(),
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

    fn inject_connected(&mut self, _peer_id: &PeerId) {}

    fn inject_disconnected(&mut self, _peer_id: &PeerId) {}

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        _conn: &ConnectionId,
        endpoint: &ConnectedPoint,
    ) {
        let mut close_connection = false;

        let (address, subnet_limit) = match endpoint {
            ConnectedPoint::Listener { send_back_addr, .. } => {
                (send_back_addr.clone(), INBOUND_PEER_COUNT_PER_SUBNET_MAX)
            }
            ConnectedPoint::Dialer { address } => (address.clone(), OUTBOUND_PEER_COUNT_PER_SUBNET_MAX),
        };

        // Get the IP for this new peer connection
        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => IpNetwork::new(ip, IPV4_SUBNET_MASK).unwrap(),
            Some(Protocol::Ip6(ip)) => IpNetwork::new(ip, IPV6_SUBNET_MASK).unwrap(),
            _ => return,
        };

        if self.ip_count.get(&ip).is_some() && PEER_COUNT_PER_IP_MAX < *self.ip_count.get(&ip).unwrap() + 1 {
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
        if PEER_COUNT_MAX < self.ipv4_count + self.ipv6_count + 1 {
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
        }

        // Send the network event to the Swarm
        if close_connection {
            /*let (peer_tx, _peer_rx) = mpsc::channel(4096);
            let peer = Arc::new(Peer::new(peer_id.clone(), peer_tx));
            self.events.push_back(NetworkEvent::PeerDisconnect(peer));*/
            todo!()
        }
    }

    fn inject_connection_closed(
        &mut self,
        peer_id: &PeerId,
        _conn: &ConnectionId,
        info: &ConnectedPoint,
    ) {
        /*let peer = self
            .peers
            .get(peer_id)
            .expect("Connection to unknown peer closed");*/
        // FIXME: A peer should actually exist here, but it's never inserted into `self.peers`.
        let peer = if let Some(peer) = self.peers.get(peer_id) { peer } else { return };

        let address = match info {
            ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr.clone(),
            ConnectedPoint::Dialer { address } => address.clone(),
        };

        // Get the IP for the closing peer connection
        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => IpNetwork::new(ip, IPV4_SUBNET_MASK).unwrap(),
            Some(Protocol::Ip6(ip)) => IpNetwork::new(ip, IPV6_SUBNET_MASK).unwrap(),
            _ => return,
        };

        /*if peer.banned {
            self.ip_ban.insert(ip, SystemTime::now() + DEFAULT_BAN_TIME);
        }*/
        // FIXME

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
        _peer_id: PeerId,
        _connection: ConnectionId,
        _msg: <Self::ProtocolsHandler as ProtocolsHandler>::OutEvent,
    ) {}

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<HandlerInEvent, LimitEvent>> {
        let ip_ban = self.ip_ban.clone();
        for (ip, time) in ip_ban {
            if time < SystemTime::now() {
                self.ip_ban.remove(&ip);
            }
        }

        // Emit custom events.
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
        }

        Poll::Pending
    }
}