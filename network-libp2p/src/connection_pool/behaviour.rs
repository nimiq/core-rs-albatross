use std::collections::{BTreeMap, BTreeSet};
use std::task::Waker;
use std::time::{Duration, Instant};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    error,
    sync::Arc,
    task::{Context, Poll},
};

use ip_network::IpNetwork;
use libp2p::swarm::DialPeerCondition;
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
use rand::seq::IteratorRandom;
use rand::thread_rng;
use tokio::time::Interval;

use crate::discovery::peer_contacts::{PeerContactBook, Services};

use super::handler::{ConnectionPoolHandler, HandlerInEvent, HandlerOutEvent};

#[derive(Clone, Debug)]
struct ConnectionPoolLimits {
    ip_count: HashMap<IpNetwork, usize>,
    ipv4_count: usize,
    ipv6_count: usize,
}

#[derive(Clone, Debug)]
struct ConnectionPoolConfig {
    peer_count_desired: usize,
    peer_count_max: usize,
    peer_count_per_ip_max: usize,
    outbound_peer_count_per_subnet_max: usize,
    inbound_peer_count_per_subnet_max: usize,
    ipv4_subnet_mask: u8,
    ipv6_subnet_mask: u8,
    dialing_count_max: usize,
    retry_down_after: Duration,
    housekeeping_interval: Duration,
}

impl Default for ConnectionPoolConfig {
    fn default() -> Self {
        Self {
            peer_count_desired: 12,
            peer_count_max: 4000,
            peer_count_per_ip_max: 20,
            outbound_peer_count_per_subnet_max: 2,
            inbound_peer_count_per_subnet_max: 10,
            ipv4_subnet_mask: 24,
            ipv6_subnet_mask: 96,
            dialing_count_max: 3,
            retry_down_after: Duration::from_secs(60 * 10), // 10 minutes
            housekeeping_interval: Duration::from_secs(60 * 2), // 2 minutes
        }
    }
}

#[derive(Clone, Debug)]
pub enum ConnectionPoolEvent {
    Disconnect { peer_id: PeerId },
}

struct ConnectionState<T> {
    dialing: BTreeSet<T>,
    connected: BTreeSet<T>,
    failed: BTreeMap<T, usize>,
    down: BTreeMap<T, Instant>,
    max_failures: usize,
    retry_down_after: Duration,
}

impl<T: Ord> ConnectionState<T> {
    fn new(max_failures: usize, retry_down_after: Duration) -> Self {
        Self {
            dialing: BTreeSet::new(),
            connected: BTreeSet::new(),
            failed: BTreeMap::new(),
            down: BTreeMap::new(),
            max_failures,
            retry_down_after,
        }
    }

    fn mark_dialing(&mut self, id: T) {
        self.dialing.insert(id);
    }

    fn mark_connected(&mut self, id: T) {
        self.dialing.remove(&id);
        self.failed.remove(&id);
        self.down.remove(&id);
        self.connected.insert(id);
    }

    fn mark_closed(&mut self, id: T) {
        self.connected.remove(&id);
    }

    fn mark_failed(&mut self, id: T) {
        self.dialing.remove(&id);

        // TODO Ignore failures if down?

        if let Some(num_attempts) = self.failed.get_mut(&id) {
            *num_attempts += 1;
            if *num_attempts >= self.max_failures {
                self.mark_down(id);
            }
        } else {
            self.failed.insert(id, 1);
        }
    }

    fn mark_down(&mut self, id: T) {
        self.failed.remove(&id);
        self.down.insert(id, Instant::now());
    }

    fn can_dial(&self, id: &T) -> bool {
        !self.dialing.contains(id) && !self.connected.contains(id) && !self.down.contains_key(id)
    }

    fn num_dialing(&self) -> usize {
        self.dialing.len()
    }

    fn num_connected(&self) -> usize {
        self.connected.len()
    }

    fn housekeeping(&mut self) {
        // Remove all down peers that we haven't dialed in a while from the `down` map to dial them again.
        let retry_down_after = self.retry_down_after;
        self.down
            .retain(|_, down_since| down_since.elapsed() < retry_down_after);
    }
}

impl<T> std::fmt::Display for ConnectionState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "connected={}, dialing={}, failed={}, down={}",
            self.connected.len(),
            self.dialing.len(),
            self.failed.len(),
            self.down.len()
        )
    }
}

pub struct ConnectionPoolBehaviour {
    contacts: Arc<RwLock<PeerContactBook>>,
    seeds: Vec<Multiaddr>,

    peers: ConnectionState<PeerId>,
    addresses: ConnectionState<Multiaddr>,

    actions: VecDeque<NetworkBehaviourAction<HandlerInEvent, ConnectionPoolEvent>>,

    active: bool,

    limits: ConnectionPoolLimits,
    config: ConnectionPoolConfig,
    banned: HashSet<IpNetwork>,
    waker: Option<Waker>,
    housekeeping_timer: Interval,
}

impl ConnectionPoolBehaviour {
    pub fn new(contacts: Arc<RwLock<PeerContactBook>>, seeds: Vec<Multiaddr>) -> Self {
        let limits = ConnectionPoolLimits {
            ip_count: HashMap::new(),
            ipv4_count: 0,
            ipv6_count: 0,
        };
        let config = ConnectionPoolConfig::default();
        let housekeeping_timer = tokio::time::interval(config.housekeeping_interval);

        Self {
            contacts,
            seeds,
            peers: ConnectionState::new(2, config.retry_down_after),
            addresses: ConnectionState::new(4, config.retry_down_after),
            actions: VecDeque::new(),
            active: false,
            limits,
            config,
            banned: HashSet::new(),
            waker: None,
            housekeeping_timer,
        }
    }

    pub fn start_connecting(&mut self) {
        self.active = true;
        self.maintain_peers();
    }

    pub fn maintain_peers(&mut self) {
        log::debug!(
            "Maintaining peers: {} | addresses: {}",
            self.peers,
            self.addresses
        );

        // Try to maintain at least `peer_count_desired` connections.
        if self.active
            && self.peers.num_connected() < self.config.peer_count_desired
            && self.peers.num_dialing() < self.config.dialing_count_max
        {
            // Dial peers from the contact book.
            for peer_id in self.choose_peers_to_dial() {
                log::debug!("Dialing peer {}", peer_id);
                self.peers.mark_dialing(peer_id);
                self.actions.push_back(NetworkBehaviourAction::DialPeer {
                    peer_id,
                    condition: DialPeerCondition::Disconnected,
                });
            }

            // Dial seeds.
            for address in self.choose_seeds_to_dial() {
                log::debug!("Dialing seed {}", address);
                self.addresses.mark_dialing(address.clone());
                self.actions
                    .push_back(NetworkBehaviourAction::DialAddress { address });
            }
        }

        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    fn choose_peers_to_dial(&self) -> Vec<PeerId> {
        let num_peers = usize::min(
            self.config.peer_count_desired - self.peers.num_connected(),
            self.config.dialing_count_max - self.peers.num_dialing(),
        );
        let contacts = self.contacts.read();
        let own_contact = contacts.get_own_contact();
        let own_peer_id = own_contact.peer_id();

        // TODO Services
        contacts
            .query(own_contact.protocols(), Services::all()) // TODO Services
            .filter_map(|contact| {
                let peer_id = contact.peer_id();
                if peer_id != own_peer_id && self.peers.can_dial(peer_id) {
                    Some(*peer_id)
                } else {
                    None
                }
            })
            .choose_multiple(&mut thread_rng(), num_peers)
    }

    fn choose_seeds_to_dial(&self) -> Vec<Multiaddr> {
        // We prefer to connect to non-seed peers. Thus, we only choose any seeds here if we're
        // not already dialing any peers and at most one seed at a time.
        if self.peers.num_dialing() > 0 || self.addresses.num_dialing() > 0 {
            return vec![];
        }

        let num_seeds = 1;
        let contacts = self.contacts.read();
        let own_addresses: HashSet<&Multiaddr> = contacts.get_own_contact().addresses().collect();
        self.seeds
            .iter()
            .filter(|address| !own_addresses.contains(address) && self.addresses.can_dial(*address))
            .cloned()
            .choose_multiple(&mut thread_rng(), num_seeds)
    }

    fn housekeeping(&mut self) {
        log::trace!("Doing housekeeping in connection pool.");

        // Disconnect peers that have negative scores.
        let contacts = self.contacts.read();
        for peer_id in &self.peers.connected {
            let peer_score = contacts.get(peer_id).map(|e| e.get_score());
            if let Some(score) = peer_score {
                if score < 0.0 {
                    self.actions
                        .push_back(NetworkBehaviourAction::GenerateEvent(
                            ConnectionPoolEvent::Disconnect { peer_id: *peer_id },
                        ));
                }
            }
        }
        drop(contacts);

        self.peers.housekeeping();
        self.addresses.housekeeping();

        self.maintain_peers();
    }
}

impl NetworkBehaviour for ConnectionPoolBehaviour {
    type ProtocolsHandler = ConnectionPoolHandler;
    type OutEvent = ConnectionPoolEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        ConnectionPoolHandler::new()
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<Multiaddr> {
        self.contacts
            .read()
            .get(peer_id)
            .map(|e| e.contact().addresses.clone())
            .unwrap_or_default()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        self.peers.mark_connected(*peer_id);
        self.maintain_peers();
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        self.peers.mark_closed(*peer_id);
        // If the connection was closed for any reason, don't dial the peer again.
        // FIXME We want to be more selective here and only mark peers as down for specific CloseReasons.
        self.peers.mark_down(*peer_id);
        self.maintain_peers();
    }

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        _conn: &ConnectionId,
        endpoint: &ConnectedPoint,
    ) {
        let address = endpoint.get_remote_address();
        log::debug!(
            "Connection established: peer_id={}, address={}",
            peer_id,
            address
        );

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
            self.actions
                .push_back(NetworkBehaviourAction::GenerateEvent(
                    ConnectionPoolEvent::Disconnect { peer_id: *peer_id },
                ));
        } else {
            // Increment peer counts per IP
            *self.limits.ip_count.entry(ip).or_insert(0) += 1;
            match ip {
                IpNetwork::V4(..) => self.limits.ipv4_count += 1,
                IpNetwork::V6(..) => self.limits.ipv6_count += 1,
            };

            self.addresses.mark_connected(address.clone());
        }
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
            IpNetwork::V4(..) => self.limits.ipv4_count = self.limits.ipv4_count.saturating_sub(1),
            IpNetwork::V6(..) => self.limits.ipv6_count = self.limits.ipv6_count.saturating_sub(1),
        };

        self.addresses.mark_closed(address.clone());
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

    fn inject_addr_reach_failure(
        &mut self,
        peer_id: Option<&PeerId>,
        addr: &Multiaddr,
        error: &dyn error::Error,
    ) {
        log::debug!(
            "Failed to reach address: {}, peer_id={:?}, error={:?}",
            addr,
            peer_id,
            error
        );
        self.addresses.mark_failed(addr.clone());
        self.maintain_peers();
    }

    fn inject_dial_failure(&mut self, peer_id: &PeerId) {
        log::debug!("Failed to dial peer: {}", peer_id);
        self.peers.mark_failed(*peer_id);
        self.maintain_peers();
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<HandlerInEvent, ConnectionPoolEvent>> {
        // Dispatch pending actions.
        if let Some(action) = self.actions.pop_front() {
            return Poll::Ready(action);
        }

        // Perform housekeeping at regular intervals.
        if self.housekeeping_timer.poll_tick(cx).is_ready() {
            self.housekeeping();
        }

        // Store waker.
        match &mut self.waker {
            Some(waker) if !waker.will_wake(cx.waker()) => *waker = cx.waker().clone(),
            None => self.waker = Some(cx.waker().clone()),
            _ => {}
        };

        Poll::Pending
    }
}
