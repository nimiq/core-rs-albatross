use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    net::IpAddr,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::StreamExt;
use instant::Instant;
use ip_network::IpNetwork;
use libp2p::{
    core::{connection::ConnectionId, multiaddr::Protocol, ConnectedPoint},
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        ConnectionHandler, DialError, IntoConnectionHandler, NetworkBehaviour,
        NetworkBehaviourAction, NotifyHandler, PollParameters,
    },
    Multiaddr, PeerId, TransportError,
};
use nimiq_macros::store_waker;
use nimiq_network_interface::{network::CloseReason, peer_info::Services};
use parking_lot::RwLock;
use rand::{seq::IteratorRandom, thread_rng};
use wasm_timer::Interval;

use super::handler::{ConnectionPoolHandler, ConnectionPoolHandlerError};
use crate::discovery::peer_contacts::PeerContactBook;

#[derive(Clone, Debug)]
struct ConnectionPoolLimits {
    ip_count: HashMap<IpAddr, usize>,
    ip_subnet_count: HashMap<IpNetwork, usize>,
    peer_count: usize,
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

/// Connection Peer information
#[derive(Clone, Debug)]
struct IpInfo {
    /// Connection IP subnet (according to a subnet mask)
    subnet_ip: Option<IpNetwork>,
    /// Connection IP address
    ip: IpAddr,
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
    /// Set of connection IDs being dialed.
    dialing: BTreeSet<T>,
    /// Set of connection IDs marked as connected.
    connected: BTreeSet<T>,
    /// Set of connection IDs marked as banned.
    banned: BTreeSet<T>,
    /// Set of connection IDs mark as failed.
    failed: BTreeMap<T, usize>,
    /// Set of connection IDs mark as down.
    /// A connection ID is marked as down if it has been marked as failed for
    /// `max_failures` or if it is marked as such manually.
    down: BTreeMap<T, Instant>,
    /// Max amount of failures allowed for a connection ID until it is marked as down.
    max_failures: usize,
    /// Time after which a connection ID would be removed from IDs marked as down.
    retry_down_after: Duration,
}

impl<T: Ord> ConnectionState<T> {
    fn new(max_failures: usize, retry_down_after: Duration) -> Self {
        Self {
            dialing: BTreeSet::new(),
            connected: BTreeSet::new(),
            banned: BTreeSet::new(),
            failed: BTreeMap::new(),
            down: BTreeMap::new(),
            max_failures,
            retry_down_after,
        }
    }

    /// Marks a connection ID as being dialed
    fn mark_dialing(&mut self, id: T) {
        self.dialing.insert(id);
    }

    /// Marks a connection ID as connected. This also implies that if the
    /// connection ID was previously marked as failed/down or being dialed it
    /// will be removed from such state.
    fn mark_connected(&mut self, id: T) {
        self.dialing.remove(&id);
        self.failed.remove(&id);
        self.down.remove(&id);
        self.connected.insert(id);
    }

    /// Marks a connection ID as closed (will be removed from the connected)
    /// set of connections.
    fn mark_closed(&mut self, id: T) {
        self.connected.remove(&id);
    }

    /// Marks a connection ID as banned. The connection ID will be also removed
    /// from the IDs marked as down or failed.
    fn mark_banned(&mut self, id: T) {
        self.failed.remove(&id);
        self.down.remove(&id);
        self.banned.insert(id);
    }

    /// Removes a connection ID from the banned set
    fn unmark_banned(&mut self, id: T) {
        self.banned.remove(&id);
    }

    /// Returns whether a connection ID is banned
    fn is_banned(&self, id: T) -> bool {
        self.banned.get(&id).is_some()
    }

    /// Marks a connection ID as failed
    ///
    /// If the peers was marked as being dialed, it will be removed from such
    /// set.
    /// Also if the connection ID has been marked more that `self.max_failures`
    /// as failed then the connection ID will be marked as down.
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

    /// Marks a connection ID as down
    ///
    /// If the connection was into the failed set, it will be removed from such
    /// set.
    fn mark_down(&mut self, id: T) {
        self.failed.remove(&id);
        self.down.insert(id, Instant::now());
    }

    /// Returns whether a specific connection ID can dial another connection ID
    fn can_dial(&self, id: &T) -> bool {
        !self.dialing.contains(id)
            && !self.connected.contains(id)
            && !self.down.contains_key(id)
            && !self.banned.contains(id)
    }

    /// Returns the number of connections being dialed
    fn num_dialing(&self) -> usize {
        self.dialing.len()
    }

    /// Returns the number of connected instances
    fn num_connected(&self) -> usize {
        self.connected.len()
    }

    /// Remove all down peers that haven't been dialed in a while from the `down`
    /// map to dial them again.
    fn housekeeping(&mut self) {
        let retry_down_after = self.retry_down_after;
        self.down
            .retain(|_, down_since| down_since.elapsed() < retry_down_after);
    }

    /// Remove all connection IDs marked as down.
    /// This will make a peer potentially able to be called again.
    fn reset_down(&mut self) {
        self.down.clear()
    }
}

impl<T> std::fmt::Display for ConnectionState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "connected={}, dialing={}, failed={}, down={}, banned={}",
            self.connected.len(),
            self.dialing.len(),
            self.failed.len(),
            self.down.len(),
            self.banned.len(),
        )
    }
}

/// Connection pool behaviour events
#[derive(Debug)]
pub enum ConnectionPoolEvent {
    /// A peer has joined
    PeerJoined { peer_id: PeerId },
}

type PoolNetworkBehaviourAction =
    NetworkBehaviourAction<ConnectionPoolEvent, ConnectionPoolHandler>;

/// Connection pool behaviour
///
/// This behaviour maintains state on wether the current peer can connect to
/// another peer. For this it maintains state about other peers such as if
/// the peer is connected, is being dialed, is down or has failed.
/// Also watches if we have received more connections than the allowed
/// configured maximum per peer, IP or subnet.
pub struct ConnectionPoolBehaviour {
    /// Peer contact book. This is the data structure where information of all
    /// known peers is store. This information includes known addresses and
    /// services of each of the peers.
    pub contacts: Arc<RwLock<PeerContactBook>>,

    /// Local (own) peer ID
    own_peer_id: PeerId,

    /// Set of seeds useful when starting to discover other peers.
    seeds: Vec<Multiaddr>,

    /// The set of services that this peer requires.
    required_services: Services,

    /// Connection state per Peer ID
    peer_ids: ConnectionState<PeerId>,

    /// Connection state per address
    addresses: ConnectionState<Multiaddr>,

    /// Queue of actions this behaviour will emit for handler execution.
    actions: VecDeque<PoolNetworkBehaviourAction>,

    /// Tells wether the connection pool behaviour is active or not
    active: bool,

    /// Counters per connection limits
    limits: ConnectionPoolLimits,

    /// Configuration for the connection pool behaviour
    config: ConnectionPoolConfig,

    /// Waker to signal when this behaviour needs to be polled again
    waker: Option<Waker>,

    /// Interval for which the connection pool housekeeping should be run
    housekeeping_timer: Interval,
}

impl ConnectionPoolBehaviour {
    pub fn new(
        contacts: Arc<RwLock<PeerContactBook>>,
        own_peer_id: PeerId,
        seeds: Vec<Multiaddr>,
        required_services: Services,
    ) -> Self {
        let limits = ConnectionPoolLimits {
            ip_count: HashMap::new(),
            ip_subnet_count: HashMap::new(),
            peer_count: 0,
        };
        let config = ConnectionPoolConfig::default();
        let housekeeping_timer = wasm_timer::Interval::new(config.housekeeping_interval);

        Self {
            contacts,
            own_peer_id,
            seeds,
            required_services,
            peer_ids: ConnectionState::new(2, config.retry_down_after),
            addresses: ConnectionState::new(4, config.retry_down_after),
            actions: VecDeque::new(),
            active: false,
            limits,
            config,
            waker: None,
            housekeeping_timer,
        }
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn get_ip_info_from_multiaddr(&self, address: &Multiaddr) -> Option<IpInfo> {
        // Get IP from multiaddress if it exists.
        match address.iter().next() {
            Some(Protocol::Ip4(ip)) => Some(IpInfo {
                subnet_ip: IpNetwork::new_truncate(ip, self.config.ipv4_subnet_mask).ok(),
                ip: IpAddr::V4(ip),
            }),
            Some(Protocol::Ip6(ip)) => Some(IpInfo {
                subnet_ip: IpNetwork::new_truncate(ip, self.config.ipv6_subnet_mask).ok(),
                ip: IpAddr::V6(ip),
            }),
            _ => None,
        }
    }

    /// Tries to maintain at least `peer_count_desired` connections.
    ///
    /// For this it will try to select peers or seeds to dial in order to
    /// achieve that many connection.
    /// Note that this only takes effect if `start_connecting` function has
    /// been previously called.
    pub fn maintain_peers(&mut self) {
        trace!(
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

    /// Tells the behaviour to start connecting to other peers.
    pub fn start_connecting(&mut self) {
        self.active = true;
        self.maintain_peers();
    }

    /// Tells the behaviour to restart connecting to other peers.
    /// For this, it clears the set of peers and addresses marked as down
    /// and tells the network to start connecting again.
    pub fn restart_connecting(&mut self) {
        // Clean up the nodes marked as down
        self.addresses.reset_down();
        self.peer_ids.reset_down();
        self.start_connecting();
    }

    /// Tells the behaviour to stop connecting to other peers.
    /// This is useful when we are sure we have no possibility of getting a
    /// connection such as in a network outage.
    fn stop_connecting(&mut self) {
        self.active = false;
    }

    /// Closes a peer connection with a reason
    ///
    /// This will take actions depending on the close reason. For instance:
    /// - The close reason `MaliciousPeer` will cause the peer to be banned.
    /// - Going offline will signal the network to stop connecting to peers.
    pub fn close_connection(&mut self, peer_id: PeerId, reason: CloseReason) {
        self.actions
            .push_back(NetworkBehaviourAction::NotifyHandler {
                peer_id,
                handler: NotifyHandler::Any,
                event: ConnectionPoolHandlerError::Other(reason),
            });
        self.wake();

        match reason {
            CloseReason::MaliciousPeer => self.ban_connection(peer_id),
            CloseReason::GoingOffline => self.stop_connecting(),
            _ => {}
        }
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

    /// This function is used to select a list of peers, based on services flag, in order to dial them.
    /// `num_peers` is used to specify how many peers are selected
    /// The number of peers returned equals num_peers unless there are less available peers
    pub fn choose_peers_to_dial_by_services(
        &self,
        services: Services,
        num_peers: usize,
    ) -> Vec<PeerId> {
        let contacts = self.contacts.read();
        let own_contact = contacts.get_own_contact();
        let own_peer_id = own_contact.peer_id();

        contacts
            .query(services)
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

        self.maintain_peers();
    }

    fn ban_connection(&mut self, peer_id: PeerId) {
        // Mark the peer ID as banned
        self.peer_ids.mark_banned(peer_id);
        debug!(%peer_id, "Banned peer");

        // Mark its addresses as banned if we have them
        if let Some(contact) = self.contacts.read().get(&peer_id) {
            let addresses = contact.addresses();
            for address in addresses {
                self.addresses.mark_banned(address.clone());
                debug!(%address, "Banned address");
            }
        }
    }

    /// Un-bans a peer connection and its IP if we have the address for such peer ID
    pub fn unban_connection(&mut self, peer_id: PeerId) {
        // Unmark the peer ID as banned
        self.peer_ids.unmark_banned(peer_id);
        debug!(%peer_id, "Un-banned peer");

        // Mark its addresses as unbanned if we have them
        if let Some(contact) = self.contacts.read().get(&peer_id) {
            let addresses = contact.addresses();
            for address in addresses {
                self.addresses.unmark_banned(address.clone());
                debug!(%address, "Un-banned address");
            }
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
            // We have more than one connection to the same peer. Deterministically
            // choose which connection to close: close the connection only if the
            // other peer ID is less than our own peer ID value.
            // Note: We don't track all of the connection IDs and if the latest
            // connection ID we get is from a peer ID with a lower value, we
            // close it. If not, we optimistically expect that the other peer
            // does it.
            if *peer_id <= self.own_peer_id {
                // Notify the handler that the connection must be closed
                self.actions
                    .push_back(NetworkBehaviourAction::NotifyHandler {
                        peer_id: *peer_id,
                        handler: NotifyHandler::One(*connection_id),
                        event: ConnectionPoolHandlerError::AlreadyConnected,
                    });
                self.wake();
            }
            return;
        }

        let address = endpoint.get_remote_address();
        let mut close_reason = None;
        if self.addresses.is_banned(address.clone()) {
            debug!(%address, "Address is banned");
            close_reason = Some(ConnectionPoolHandlerError::BannedIp);
        } else if self.peer_ids.is_banned(*peer_id) {
            debug!(%peer_id, "Peer is banned");
            close_reason = Some(ConnectionPoolHandlerError::BannedPeer);
        }

        // Get IP from multiaddress if it exists.
        let ip_info = self.get_ip_info_from_multiaddr(address);

        // If we have an IP, check connection limits per IP.
        if let Some(ip_info) = ip_info.clone() {
            if self.config.peer_count_per_ip_max
                < self
                    .limits
                    .ip_count
                    .get(&ip_info.ip)
                    .unwrap_or(&0)
                    .saturating_add(1)
            {
                // Subnet mask
                debug!(ip=%ip_info.ip, limit=self.config.peer_count_per_ip_max, "Max peer connections per IP limit reached");
                close_reason = Some(ConnectionPoolHandlerError::MaxPeerPerIPConnectionsReached);
            }

            // If we have the subnet IP, check connection limits per subnet
            if let Some(subnet_ip) = ip_info.subnet_ip {
                if self.config.peer_count_per_subnet_max
                    < self
                        .limits
                        .ip_subnet_count
                        .get(&subnet_ip)
                        .unwrap_or(&0)
                        .saturating_add(1)
                {
                    // Subnet mask
                    debug!(%subnet_ip, limit=self.config.peer_count_per_subnet_max, "Max peer connections per IP subnet limit reached");
                    close_reason = Some(ConnectionPoolHandlerError::MaxSubnetConnectionsReached);
                }
            }
        }

        // Check for the maximum peer count limit
        if self.config.peer_count_max < self.limits.peer_count.saturating_add(1) {
            debug!(
                connections = self.limits.peer_count,
                "Max peer connections limit reached"
            );
            close_reason = Some(ConnectionPoolHandlerError::MaxPeerConnectionsReached);
        }

        if let Some(ip_info) = ip_info {
            if close_reason.is_none() {
                // Increment peer counts per IP if we are not going to close the connection
                if let Some(subnet_ip) = ip_info.subnet_ip {
                    let value = self.limits.ip_subnet_count.entry(subnet_ip).or_insert(0);
                    *value = value.saturating_add(1);
                }

                let value = self.limits.ip_count.entry(ip_info.ip).or_insert(0);
                *value = value.saturating_add(1);

                self.limits.peer_count = self.limits.peer_count.saturating_add(1);
            }
        }

        if let Some(close_reason) = close_reason {
            // Notify the handler that the connection must be closed
            self.actions
                .push_back(NetworkBehaviourAction::NotifyHandler {
                    peer_id: *peer_id,
                    handler: NotifyHandler::One(*connection_id),
                    event: close_reason,
                });
            self.wake();
            return;
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

        // Get IP from multiaddress if it exists.
        let ip_info = self.get_ip_info_from_multiaddr(address);

        // Decrement IP counters if needed
        if let Some(ip_info) = ip_info {
            let value = self.limits.ip_count.entry(ip_info.ip).or_insert(1);
            *value = value.saturating_sub(1);
            if *self.limits.ip_count.get(&ip_info.ip).unwrap() == 0 {
                self.limits.ip_count.remove(&ip_info.ip);
            }

            if let Some(subnet_ip) = ip_info.subnet_ip {
                let value = self.limits.ip_subnet_count.entry(subnet_ip).or_insert(1);
                *value = value.saturating_sub(1);
                if *self.limits.ip_subnet_count.get(&subnet_ip).unwrap() == 0 {
                    self.limits.ip_subnet_count.remove(&subnet_ip);
                }
            }
        }

        self.limits.peer_count = self.limits.peer_count.saturating_sub(1);

        self.addresses.mark_closed(address.clone());
        self.peer_ids.mark_closed(*peer_id);
        // If the connection was closed for any reason, don't dial the peer again.
        // FIXME We want to be more selective here and only mark peers as down for specific CloseReasons.
        self.peer_ids.mark_down(*peer_id);
        self.addresses.mark_down(address.clone());

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
        let error_msg = match error {
            DialError::Transport(errors) => {
                let str = errors
                    .iter()
                    .map(|(address, error)| {
                        let mut address = address.clone();

                        // XXX Cut off public key
                        address.pop();

                        let err = match error {
                            TransportError::MultiaddrNotSupported(_) => {
                                "Multiaddr not supported".to_string()
                            }
                            TransportError::Other(e) => e.to_string(),
                        };

                        format!("{} => {}", address, err)
                    })
                    .collect::<Vec<String>>()
                    .join(", ");

                format!("No transport: {}", str)
            }
            DialError::ConnectionIo(error) => error.to_string(),
            e => e.to_string(),
        };

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

                debug!(%peer_id, error = error_msg, "Failed to dial peer");
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
