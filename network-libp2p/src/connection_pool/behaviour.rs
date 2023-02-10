use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::StreamExt;
use instant::{Instant, SystemTime};
use ip_network::IpNetwork;
use libp2p::swarm::dial_opts::PeerCondition;
use libp2p::{
    core::{connection::ConnectionId, multiaddr::Protocol, ConnectedPoint},
    swarm::{
        dial_opts::DialOpts, CloseConnection, ConnectionHandler, DialError, IntoConnectionHandler,
        NetworkBehaviour, NetworkBehaviourAction, PollParameters,
    },
    Multiaddr, PeerId,
};
use parking_lot::RwLock;
use rand::seq::IteratorRandom;
use rand::thread_rng;
use wasm_timer::Interval;

use nimiq_macros::store_waker;

use crate::discovery::peer_contacts::{PeerContactBook, Services};

use super::handler::ConnectionPoolHandler;

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
    peer_count_per_subnet_max: usize,
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
            peer_count_per_subnet_max: 20,
            ipv4_subnet_mask: 24,
            ipv6_subnet_mask: 96,
            dialing_count_max: 3,
            retry_down_after: Duration::from_secs(60 * 10), // 10 minutes
            housekeeping_interval: Duration::from_secs(60 * 2), // 2 minutes
        }
    }
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

#[derive(Debug)]
pub enum ConnectionPoolEvent {
    PeerJoined { peer_id: PeerId },
}

type PoolNetworkBehaviourAction =
    NetworkBehaviourAction<ConnectionPoolEvent, ConnectionPoolHandler>;

pub struct ConnectionPoolBehaviour {
    pub contacts: Arc<RwLock<PeerContactBook>>,
    seeds: Vec<Multiaddr>,

    required_services: Services,

    peer_ids: ConnectionState<PeerId>,
    addresses: ConnectionState<Multiaddr>,

    actions: VecDeque<PoolNetworkBehaviourAction>,

    active: bool,

    limits: ConnectionPoolLimits,
    config: ConnectionPoolConfig,
    banned: HashMap<IpNetwork, SystemTime>,
    waker: Option<Waker>,
    housekeeping_timer: Interval,
}

impl ConnectionPoolBehaviour {
    pub fn new(
        contacts: Arc<RwLock<PeerContactBook>>,
        seeds: Vec<Multiaddr>,
        required_services: Services,
    ) -> Self {
        let limits = ConnectionPoolLimits {
            ip_count: HashMap::new(),
            ipv4_count: 0,
            ipv6_count: 0,
        };
        let config = ConnectionPoolConfig::default();
        let housekeeping_timer = wasm_timer::Interval::new(config.housekeeping_interval);

        Self {
            contacts,
            seeds,
            required_services,
            peer_ids: ConnectionState::new(2, config.retry_down_after),
            addresses: ConnectionState::new(4, config.retry_down_after),
            actions: VecDeque::new(),
            active: false,
            limits,
            config,
            banned: HashMap::new(),
            waker: None,
            housekeeping_timer,
        }
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    pub fn maintain_peers(&mut self) {
        debug!(
            peer_ids = %self.peer_ids,
            addresses = %self.addresses,
            "Maintaining peers"
        );

        // Try to maintain at least `peer_count_desired` connections.
        if self.active
            && self.peer_ids.num_connected() < self.config.peer_count_desired
            && self.peer_ids.num_dialing() < self.config.dialing_count_max
        {
            // Dial peers from the contact book.
            for peer_id in self.choose_peers_to_dial() {
                debug!(%peer_id, "Dialing peer");
                self.peer_ids.mark_dialing(peer_id);
                let handler = self.new_handler();
                self.actions.push_back(NetworkBehaviourAction::Dial {
                    opts: DialOpts::peer_id(peer_id)
                        .condition(PeerCondition::Disconnected)
                        .build(),
                    handler,
                });
            }

            // Dial seeds.
            for address in self.choose_seeds_to_dial() {
                debug!(%address, "Dialing seed");
                self.addresses.mark_dialing(address.clone());
                let handler = self.new_handler();
                self.actions.push_back(NetworkBehaviourAction::Dial {
                    opts: DialOpts::unknown_peer_id().address(address).build(),
                    handler,
                });
            }
        }

        self.wake();
    }

    pub fn start_connecting(&mut self) {
        self.active = true;
        self.maintain_peers();
    }

    fn choose_peers_to_dial(&self) -> Vec<PeerId> {
        let num_peers = usize::min(
            self.config.peer_count_desired - self.peer_ids.num_connected(),
            self.config.dialing_count_max - self.peer_ids.num_dialing(),
        );
        let contacts = self.contacts.read();
        let own_contact = contacts.get_own_contact();
        let own_peer_id = own_contact.peer_id();

        contacts
            .query(self.required_services)
            .filter_map(|contact| {
                let peer_id = contact.peer_id();
                if peer_id != own_peer_id
                    && self.peer_ids.can_dial(peer_id)
                    && contact.addresses().count() > 0
                {
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
        if self.peer_ids.num_dialing() > 0 || self.addresses.num_dialing() > 0 {
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
        trace!("Doing housekeeping in connection pool");

        // Disconnect peers that have negative scores.
        let contacts = self.contacts.read();
        for peer_id in &self.peer_ids.connected {
            let peer_score = contacts.get(peer_id).map(|e| e.get_score());
            if let Some(score) = peer_score {
                if score < 0.0 {
                    info!(%peer_id, score, "Peer has a negative score");
                }
            }
        }
        drop(contacts);

        self.peer_ids.housekeeping();
        self.addresses.housekeeping();

        for (ip, time) in self.banned.clone() {
            if time < SystemTime::now() {
                self.banned.remove(&ip);
            }
        }

        self.maintain_peers();
    }

    pub fn _ban_ip(&mut self, ip: IpNetwork) {
        if self
            .banned
            .insert(ip, SystemTime::now() + Duration::from_secs(60 * 10)) // 10 minutes
            .is_none()
        {
            debug!(%ip, "IP added to banned set of peers");
        } else {
            debug!(%ip, "IP already part of banned set of peers");
        }
    }

    pub fn _unban_ip(&mut self, ip: IpNetwork) {
        if self.banned.remove(&ip).is_some() {
            debug!(%ip, "IP removed from banned set of peers");
        } else {
            debug!(%ip, "IP was not part of banned set of peers");
        }
    }
}

impl NetworkBehaviour for ConnectionPoolBehaviour {
    type ConnectionHandler = ConnectionPoolHandler;
    type OutEvent = ConnectionPoolEvent;

    fn new_handler(&mut self) -> Self::ConnectionHandler {
        ConnectionPoolHandler::default()
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<Multiaddr> {
        self.contacts
            .read()
            .get(peer_id)
            .map(|e| e.contact().addresses.clone())
            .unwrap_or_default()
    }

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        endpoint: &ConnectedPoint,
        failed_addresses: Option<&Vec<Multiaddr>>,
        other_established: usize,
    ) {
        let address = endpoint.get_remote_address();

        debug!(
            %peer_id,
            %address,
            ?endpoint,
            "Connection established",
        );

        // Mark failed addresses as such.
        if let Some(addresses) = failed_addresses {
            for address in addresses {
                self.addresses.mark_failed(address.clone());
            }
        }

        // Ignore connection if another connection to this peer already exists.
        // TODO Do we still want to subject it to the IP limit checks?
        if other_established > 0 {
            debug!(
                %peer_id,
                connections = other_established,
                "Already have connections established to this peer",
            );
            return;
        }

        // Get IP from multiaddress if it exists.
        let ip = match address.iter().next() {
            Some(Protocol::Ip4(ip)) => {
                IpNetwork::new_truncate(ip, self.config.ipv4_subnet_mask).ok()
            }
            Some(Protocol::Ip6(ip)) => {
                IpNetwork::new_truncate(ip, self.config.ipv6_subnet_mask).ok()
            }
            _ => None,
        };

        // If we have an IP, check connection limits per IP/subnet.
        if let Some(ip) = ip {
            let mut close_connection = false;

            if self.banned.get(&ip).is_some() {
                debug!(%ip, "IP is banned");
                close_connection = true;
            }

            if self.config.peer_count_per_ip_max
                < self
                    .limits
                    .ip_count
                    .get(&ip)
                    .unwrap_or(&0)
                    .saturating_add(1)
            {
                debug!(%ip, "Max peer connections per IP limit reached");
                close_connection = true;
            }

            if ip.is_ipv4()
                && (self.config.peer_count_per_subnet_max
                    < self.limits.ipv4_count.saturating_add(1))
            {
                debug!("Max peer connections per IPv4 subnet limit reached");
                close_connection = true;
            }

            if ip.is_ipv6()
                && (self.config.peer_count_per_subnet_max
                    < self.limits.ipv6_count.saturating_add(1))
            {
                debug!("Max peer connections per IPv6 subnet limit reached");
                close_connection = true;
            }

            if self.config.peer_count_max
                < self
                    .limits
                    .ipv4_count
                    .saturating_add(self.limits.ipv6_count)
                    .saturating_add(1)
            {
                debug!("Max peer connections limit reached");
                close_connection = true;
            }

            if close_connection {
                self.actions
                    .push_back(NetworkBehaviourAction::CloseConnection {
                        peer_id: *peer_id,
                        connection: CloseConnection::One(*connection_id),
                    });
                self.wake();
                return;
            }

            // Increment peer counts per IP
            let value = self.limits.ip_count.entry(ip).or_insert(0);
            *value = value.saturating_add(1);
            match ip {
                IpNetwork::V4(..) => {
                    self.limits.ipv4_count = self.limits.ipv4_count.saturating_add(1)
                }
                IpNetwork::V6(..) => {
                    self.limits.ipv6_count = self.limits.ipv6_count.saturating_add(1)
                }
            };
        }

        // Peer is connected, mark it as such.
        self.peer_ids.mark_connected(*peer_id);
        self.addresses.mark_connected(address.clone());

        self.actions
            .push_back(NetworkBehaviourAction::GenerateEvent(
                ConnectionPoolEvent::PeerJoined { peer_id: *peer_id },
            ));
        self.wake();

        self.maintain_peers();
    }

    fn inject_connection_closed(
        &mut self,
        peer_id: &PeerId,
        _conn: &ConnectionId,
        endpoint: &ConnectedPoint,
        _handler: <Self::ConnectionHandler as IntoConnectionHandler>::Handler,
        remaining_established: usize,
    ) {
        // Check there are no more remaining connections to this peer
        if remaining_established > 0 {
            return;
        }

        let address = endpoint.get_remote_address();

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
        let value = self.limits.ip_count.entry(ip).or_insert(1);
        *value = value.saturating_sub(1);
        if *self.limits.ip_count.get(&ip).unwrap() == 0 {
            self.limits.ip_count.remove(&ip);
        }

        match ip {
            IpNetwork::V4(..) => self.limits.ipv4_count = self.limits.ipv4_count.saturating_sub(1),
            IpNetwork::V6(..) => self.limits.ipv6_count = self.limits.ipv6_count.saturating_sub(1),
        };

        self.addresses.mark_closed(address.clone());
        self.peer_ids.mark_closed(*peer_id);
        // If the connection was closed for any reason, don't dial the peer again.
        // FIXME We want to be more selective here and only mark peers as down for specific CloseReasons.
        self.peer_ids.mark_down(*peer_id);

        self.maintain_peers();
    }

    fn inject_event(
        &mut self,
        _peer_id: PeerId,
        _connection: ConnectionId,
        _event: <<Self::ConnectionHandler as IntoConnectionHandler>::Handler as ConnectionHandler>::OutEvent,
    ) {
    }

    fn inject_dial_failure(
        &mut self,
        peer_id: Option<PeerId>,
        _handler: Self::ConnectionHandler,
        error: &DialError,
    ) {
        match error {
            DialError::Banned
            | DialError::ConnectionLimit(_)
            | DialError::LocalPeerId
            | DialError::InvalidPeerId(_)
            | DialError::WrongPeerId { .. }
            | DialError::Aborted
            | DialError::ConnectionIo(_)
            | DialError::Transport(_)
            | DialError::NoAddresses => {
                let peer_id = match peer_id {
                    Some(id) => id,
                    // Not interested in dial failures to unknown peers right now.
                    None => return,
                };

                debug!(%peer_id, %error, "Failed to dial peer");
                self.peer_ids.mark_failed(peer_id);
                self.maintain_peers();
            }
            DialError::DialPeerConditionFalse(
                PeerCondition::Disconnected | PeerCondition::NotDialing,
            ) => {
                // We might (still) be connected, or about to be connected, thus do not report the
                // failure.
            }
            DialError::DialPeerConditionFalse(PeerCondition::Always) => {
                unreachable!("DialPeerCondition::Always can not trigger DialPeerConditionFalse.");
            }
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler>> {
        // Dispatch pending actions.
        if let Some(action) = self.actions.pop_front() {
            return Poll::Ready(action);
        }

        // Perform housekeeping at regular intervals.
        if self.housekeeping_timer.poll_next_unpin(cx).is_ready() {
            self.housekeeping();
        }

        store_waker!(self, waker, cx);

        Poll::Pending
    }
}
