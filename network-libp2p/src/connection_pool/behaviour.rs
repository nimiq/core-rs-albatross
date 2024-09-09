use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    net::IpAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{future::BoxFuture, Future, FutureExt, StreamExt};
use instant::Instant;
use ip_network::IpNetwork;
use libp2p::{
    core::{multiaddr::Protocol, transport::PortUse, ConnectedPoint, Endpoint},
    swarm::{
        behaviour::{ConnectionClosed, ConnectionEstablished, DialFailure},
        dial_opts::{DialOpts, PeerCondition},
        dummy, CloseConnection, ConnectionDenied, ConnectionId, DialError, FromSwarm,
        NetworkBehaviour, THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, TransportError,
};
use nimiq_network_interface::{network::CloseReason, peer_info::Services};
use nimiq_time::{interval, sleep_until, Interval};
use nimiq_utils::WakerExt as _;
use parking_lot::RwLock;
use rand::{seq::IteratorRandom, thread_rng};
use void::Void;

use super::Error;
use crate::discovery::{handler, peer_contacts::PeerContactBook};

/// Current state of connections and peers for connection limits
#[derive(Clone, Debug)]
struct Limits {
    /// Number of peers connected per IP
    ip_count: HashMap<IpAddr, usize>,
    /// Number of peers connected per IP subnet
    ip_subnet_count: HashMap<IpNetwork, usize>,
    /// Total peer count
    peer_count: usize,
}

/// Connection pool behaviour configuration
#[derive(Clone, Debug)]
struct Config {
    /// Desired count of peers
    desired_peer_count: usize,
    /// Maximum count of peers
    peer_count_max: usize,
    /// Maximum peer count per IP
    peer_count_per_ip_max: usize,
    /// Maximum peer count per subnet
    peer_count_per_subnet_max: usize,
    /// IPv4 subnet prefix length to apply to detect same connections for the same subnet
    ipv4_subnet_prefix_len: u8,
    /// IPv6 subnet prefix length to apply to detect same connections for the same subnet
    ipv6_subnet_prefix_len: u8,
    /// Maximum number of peer dialings that can be in progress
    dialing_count_max: usize,
    /// Duration after which a peer will be re-dialed after it has been marked as down
    retry_down_after: Duration,
    /// Interval duration for peer connections housekeeping
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

impl Default for Config {
    fn default() -> Self {
        Self {
            desired_peer_count: 12,
            peer_count_max: 4000,
            peer_count_per_ip_max: 20,
            peer_count_per_subnet_max: 20,
            ipv4_subnet_prefix_len: 24,
            ipv6_subnet_prefix_len: 96,
            dialing_count_max: 3,
            retry_down_after: Duration::from_secs(60 * 10), // 10 minutes
            housekeeping_interval: Duration::from_secs(60 * 2), // 2 minutes
        }
    }
}

/// State of all of the connections the network has, like
/// connected peers, peers being dialed, peers with failed dial attempts
/// peers that are down or banned.
struct ConnectionState<T> {
    /// Set of connection IDs being dialed.
    dialing: BTreeSet<T>,
    /// Set of connection IDs marked as connected.
    connected: BTreeMap<T, Option<Services>>,
    /// Set of connection IDs marked as banned.
    banned: BTreeSet<T>,
    /// List of subsequent banned peers with their unban deadlines in ascending order.
    unban_deadlines: VecDeque<(T, Instant)>,
    /// Deadline for first peer that can be unbanned.
    unban_timeout: Option<(T, BoxFuture<'static, ()>)>,
    /// The time that needs to pass to unban a banned peer.
    ban_time: Duration,
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
    /// Desired number of connections. When the number of connections is below this number,
    /// the `housekeeping` will retry the down nodes after 1s instead of `retry_down_after`.
    desired_connections: usize,
    /// The set of services that this peer requires.
    required_services: Services,
    /// Waker used for the next poll
    waker: Option<Waker>,
}

impl<T: Clone + Ord + Unpin> Future for ConnectionState<T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Some((id, sleep)) = self.unban_timeout.as_mut() {
            if sleep.poll_unpin(cx).is_ready() {
                let id = id.clone();
                self.banned.remove(&id);
                self.unban_timeout = None;

                // While we are unbanning, try to unban more peers if their deadline also have been reached.
                // Otherwise schedule a sleep until for the first peer that can be unbanned.
                while let Some((next_id, deadline)) = self.unban_deadlines.pop_front() {
                    if Instant::now() >= deadline {
                        self.banned.remove(&next_id);
                    } else {
                        self.unban_timeout = Some((next_id, Box::pin(sleep_until(deadline))));
                        break;
                    }
                }
            } else {
                break;
            }
        }

        if self.unban_timeout.is_none() {
            self.waker.store_waker(cx);
        }

        Poll::Pending
    }
}

impl<T: Clone + Ord> ConnectionState<T> {
    fn new(
        max_failures: usize,
        retry_down_after: Duration,
        desired_connections: usize,
        required_services: Services,
        ban_time: Duration,
    ) -> Self {
        Self {
            dialing: BTreeSet::new(),
            connected: BTreeMap::new(),
            banned: BTreeSet::new(),
            ban_time,
            unban_deadlines: VecDeque::new(),
            unban_timeout: None,
            failed: BTreeMap::new(),
            down: BTreeMap::new(),
            max_failures,
            retry_down_after,
            desired_connections,
            required_services,
            waker: None,
        }
    }

    /// Marks a connection ID as being dialed
    fn mark_dialing(&mut self, id: T) {
        self.dialing.insert(id);
    }

    /// Marks a connection ID as connected. This also implies that if the
    /// connection ID was previously marked as failed/down or being dialed it
    /// will be removed from such state.
    fn mark_connected(&mut self, id: T, services: Option<Services>) {
        self.dialing.remove(&id);
        self.failed.remove(&id);
        self.down.remove(&id);
        self.connected.insert(id, services);
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
        self.banned.insert(id.clone());

        let unban_deadline = Instant::now() + self.ban_time;
        // If no peer is scheduled to be unbanned, put is as the first-to-be unbanned peer.
        // Otherwise queue the peer such that it will be scheduled later when the peer is the first in line.
        if self.unban_timeout.is_none() {
            self.unban_timeout = Some((id, Box::pin(sleep_until(unban_deadline))));
            self.waker.wake();
        } else {
            self.unban_deadlines.push_back((id.clone(), unban_deadline));
        }
    }

    /// Returns whether a connection ID is banned
    fn is_banned(&self, id: T) -> bool {
        self.banned.contains(&id)
    }

    /// Marks a connection ID as failed
    ///
    /// If the peers was marked as being dialed, it will be removed from such
    /// set.
    /// Also if the connection ID has been marked more that `self.max_failures`
    /// as failed then the connection ID will be marked as down.
    fn mark_failed(&mut self, id: T) {
        let dialing = self.dialing.remove(&id);
        if dialing && self.connected.contains_key(&id) {
            // We attempted a dial to an already connected peer.
            // This could happen dialing a peer that is also dialing us
            return;
        }

        // Peer is already marked as down. There is no point in incrementing the
        // number of failed attempts
        if self.down.contains_key(&id) {
            return;
        }

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
            && !self.connected.contains_key(id)
            && !self.down.contains_key(id)
            && !self.banned.contains(id)
    }

    /// Returns the number of connections being dialed
    fn num_dialing(&self) -> usize {
        self.dialing.len()
    }

    /// Returns the number of connected instances
    /// If `filter_required_services` is set, the number of instances is filtered by its known services and are only
    /// returned if they contain our required services.
    fn num_connected(&self, filter_required_services: bool) -> usize {
        if filter_required_services {
            self.connected
                .iter()
                .filter(|(_key, peer_services)| {
                    if let Some(peer_services) = peer_services {
                        return peer_services.contains(self.required_services);
                    }
                    false
                })
                .count()
        } else {
            self.connected.len()
        }
    }

    /// Remove all down peers that haven't been dialed in a while from the `down`
    /// map to dial them again. If the number of connections is less than the desired number
    /// of connections, this happens for every connection marked as down after 1s, if not then
    /// `self.retry_after_down is used`.
    fn housekeeping(&mut self) {
        let retry_down_after = if self.num_connected(true) < self.desired_connections {
            Duration::from_secs(1)
        } else {
            self.retry_down_after
        };
        self.down
            .retain(|_, down_since| down_since.elapsed() < retry_down_after);
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

type PoolToSwarm = ToSwarm<Void, Void>;

/// Connection pool behaviour
///
/// This behaviour maintains state on whether the current peer can connect to
/// another peer. For this it maintains state about other peers such as if
/// the peer is connected, is being dialed, is down or has failed.
/// Also watches if we have received more connections than the allowed
/// configured maximum per peer, IP or subnet.
pub struct Behaviour {
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
    actions: VecDeque<PoolToSwarm>,

    /// Tells whether the connection pool behaviour is active or not
    active: bool,

    /// Counters per connection limits
    limits: Limits,

    /// Configuration for the connection pool behaviour
    config: Config,

    /// Waker to signal when this behaviour needs to be polled again
    waker: Option<Waker>,

    /// Interval for which the connection pool housekeeping should be run
    housekeeping_timer: Interval,
}

impl Behaviour {
    pub fn new(
        contacts: Arc<RwLock<PeerContactBook>>,
        own_peer_id: PeerId,
        seeds: Vec<Multiaddr>,
        required_services: Services,
        desired_peer_count: usize,
    ) -> Self {
        let limits = Limits {
            ip_count: HashMap::new(),
            ip_subnet_count: HashMap::new(),
            peer_count: 0,
        };
        let config = Config {
            desired_peer_count,
            ..Default::default()
        };
        let housekeeping_timer = interval(config.housekeeping_interval);

        Self {
            contacts,
            own_peer_id,
            seeds,
            required_services,
            peer_ids: ConnectionState::new(
                2,
                config.retry_down_after,
                desired_peer_count,
                required_services,
                Duration::from_secs(60 * 10), // 10 minutes
            ),
            addresses: ConnectionState::new(
                4,
                config.retry_down_after,
                desired_peer_count,
                required_services,
                Duration::from_secs(60 * 10), // 10 minutes
            ),
            actions: VecDeque::new(),
            active: false,
            limits,
            config,
            waker: None,
            housekeeping_timer,
        }
    }

    fn get_ip_info_from_multiaddr(&self, address: &Multiaddr) -> Option<IpInfo> {
        // Get IP from multiaddress if it exists.
        match address.iter().next() {
            Some(Protocol::Ip4(ip)) => Some(IpInfo {
                subnet_ip: IpNetwork::new_truncate(ip, self.config.ipv4_subnet_prefix_len).ok(),
                ip: IpAddr::V4(ip),
            }),
            Some(Protocol::Ip6(ip)) => Some(IpInfo {
                subnet_ip: IpNetwork::new_truncate(ip, self.config.ipv6_subnet_prefix_len).ok(),
                ip: IpAddr::V6(ip),
            }),
            _ => None,
        }
    }

    /// Tries to maintain at least `desired_peer_count` connections.
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

        // If we are active and have less connections than the desired amount
        // and we are not dialing anyone, it is most likely because we went down
        // (i.e. we are or were offline).
        // Make sure seeds and peers are dial-able again ASAP by just calling
        // the addresses and peer IDs housekeeping since it has a mechanism to
        // reset the connections marked as down after 1s if the number of connections
        // is less than the desired peer count
        if self.active
            && self.peer_ids.num_connected(true) < self.config.desired_peer_count
            && self.peer_ids.num_dialing() + self.addresses.num_dialing() == 0
        {
            self.addresses.housekeeping();
            self.peer_ids.housekeeping();
        }

        // Try to maintain at least `desired_peer_count` connections.
        // Note: when counting dialing IDs we have to account for peer IDs and
        // addresses (seeds may only be in the `addresses` set).
        if self.active
            && self.peer_ids.num_connected(true) < self.config.desired_peer_count
            && self.peer_ids.num_dialing() + self.addresses.num_dialing()
                < self.config.dialing_count_max
        {
            // Dial peers from the contact book.
            for peer_id in self.choose_peers_to_dial() {
                self.peer_ids.mark_dialing(peer_id);
                self.actions.push_back(ToSwarm::Dial {
                    opts: DialOpts::peer_id(peer_id)
                        .condition(PeerCondition::Disconnected)
                        .build(),
                });
            }

            // Dial seeds.
            for address in self.choose_seeds_to_dial() {
                debug!(%address, "Dialing seed");
                self.addresses.mark_dialing(address.clone());
                self.actions.push_back(ToSwarm::Dial {
                    opts: DialOpts::unknown_peer_id().address(address).build(),
                });
            }
        }

        self.waker.wake();
    }

    /// Tells the behaviour to start connecting to other peers.
    pub fn start_connecting(&mut self) {
        self.active = true;
        self.maintain_peers();
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
    /// - The close reason `MaliciousPeer` will cause the peer to be banned for a fixed amount of time.
    /// - Going offline will signal the network to stop connecting to peers.
    pub fn close_connection(&mut self, peer_id: PeerId, reason: CloseReason) {
        self.actions.push_back(ToSwarm::CloseConnection {
            peer_id,
            connection: CloseConnection::All,
        });
        self.waker.wake();

        match reason {
            CloseReason::MaliciousPeer => self.ban_connection(peer_id),
            CloseReason::GoingOffline => self.stop_connecting(),
            _ => {}
        }
    }

    fn choose_peers_to_dial(&self) -> Vec<PeerId> {
        let num_peers = usize::min(
            self.config.desired_peer_count - self.peer_ids.num_connected(true),
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
        let own_contact = contacts.get_own_contact();
        let own_addresses: HashSet<&Multiaddr> = own_contact.addresses().collect();
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
        for peer_id in self.peer_ids.connected.keys() {
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

        // Mark its outer protocol address as banned if we have it
        if let Some(contact) = self.contacts.read().get(&peer_id) {
            if let Some(outer_protocol_address) = contact.get_outer_protocol_address() {
                debug!(address = %outer_protocol_address, "Banned address");
                self.addresses.mark_banned(outer_protocol_address);
            }
        }
    }

    fn on_connection_established(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        endpoint: &ConnectedPoint,
        failed_addresses: &[Multiaddr],
        other_established: usize,
    ) {
        let address = endpoint.get_remote_address();

        // Mark failed addresses as such.
        for address in failed_addresses {
            self.addresses.mark_failed(address.clone());
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
                self.actions.push_back(ToSwarm::CloseConnection {
                    peer_id: *peer_id,
                    connection: CloseConnection::One(*connection_id),
                });
                self.waker.wake();
            }
            return;
        }

        // Get IP from multiaddress if it exists.
        let ip_info = self.get_ip_info_from_multiaddr(address);
        if let Some(ip_info) = ip_info {
            // Increment peer counts per IP
            if let Some(subnet_ip) = ip_info.subnet_ip {
                let value = self.limits.ip_subnet_count.entry(subnet_ip).or_insert(0);
                *value = value.saturating_add(1);
            }

            let value = self.limits.ip_count.entry(ip_info.ip).or_insert(0);
            *value = value.saturating_add(1);

            self.limits.peer_count = self.limits.peer_count.saturating_add(1);
        }

        // Peer is connected, mark it as such.
        let peer_services = self
            .contacts
            .read()
            .get(peer_id)
            .map(|contact| contact.services());
        self.peer_ids.mark_connected(*peer_id, peer_services);
        self.addresses
            .mark_connected(address.clone(), peer_services);

        self.waker.wake();

        self.maintain_peers();
    }

    fn on_connection_closed(
        &mut self,
        peer_id: &PeerId,
        endpoint: &ConnectedPoint,
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
        self.peer_ids.mark_down(*peer_id);
        self.addresses.mark_down(address.clone());

        self.maintain_peers();
    }

    fn on_dial_failure(&mut self, peer_id: Option<PeerId>, error: &DialError) {
        let error_msg = match error {
            DialError::Transport(errors) => {
                errors
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
                    .join(", ")
            }
            e => e.to_string(),
        };

        match error {
            DialError::LocalPeerId { .. }
            | DialError::WrongPeerId { .. }
            | DialError::Aborted
            | DialError::NoAddresses
            | DialError::Denied { .. } => {
                // We're not interested in dial failures to unknown peers right now.
                let Some(peer_id) = peer_id else {
                    return;
                };

                debug!(%peer_id, error = error_msg, "Failed to dial peer");

                self.peer_ids.mark_failed(peer_id);
                self.maintain_peers();
            }
            DialError::Transport(addresses) => {
                let peer_str = match peer_id {
                    Some(id) => id.to_string(),
                    None => "None".to_string(),
                };
                debug!(peer_id = %peer_str, error = error_msg, "Failed to dial");

                if let Some(peer_id) = peer_id {
                    self.peer_ids.mark_failed(peer_id);
                }
                for (address, _) in addresses {
                    self.addresses.mark_failed(address.clone());
                }
                self.maintain_peers();
            }
            DialError::DialPeerConditionFalse(
                PeerCondition::Disconnected
                | PeerCondition::NotDialing
                | PeerCondition::DisconnectedAndNotDialing,
            ) => {
                // We might (still) be connected, or about to be connected, thus do not report the
                // failure.
            }
            DialError::DialPeerConditionFalse(PeerCondition::Always) => {
                unreachable!("DialPeerCondition::Always can not trigger DialPeerConditionFalse");
            }
        }
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Void;

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                failed_addresses,
                other_established,
            }) => {
                self.on_connection_established(
                    &peer_id,
                    &connection_id,
                    endpoint,
                    failed_addresses,
                    other_established,
                );
            }
            FromSwarm::ConnectionClosed(ConnectionClosed {
                peer_id,
                endpoint,
                remaining_established,
                ..
            }) => {
                self.on_connection_closed(&peer_id, endpoint, remaining_established);
            }
            FromSwarm::DialFailure(DialFailure { peer_id, error, .. }) => {
                self.on_dial_failure(peer_id, error)
            }
            _ => {}
        }
    }

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        void::unreachable(event)
    }

    fn handle_pending_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        _addresses: &[Multiaddr],
        _effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        let peer_id = match maybe_peer {
            None => return Ok(vec![]),
            Some(peer) => peer,
        };

        Ok(self
            .contacts
            .read()
            .get_addresses(&peer_id)
            .unwrap_or_default())
    }

    fn handle_pending_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        if let Some(outer_protocol_address) = handler::outer_protocol_address(remote_addr) {
            if self.addresses.is_banned(outer_protocol_address) {
                debug!(%remote_addr, "Address is banned");
                return Err(ConnectionDenied::new(Error::BannedIp));
            }
        }

        // Get IP from multiaddress if it exists.
        let ip_info = self.get_ip_info_from_multiaddr(remote_addr);

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
                return Err(ConnectionDenied::new(Error::MaxPeerPerIPConnectionsReached));
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
                    return Err(ConnectionDenied::new(Error::MaxSubnetConnectionsReached));
                }
            }
        }

        // Check for the maximum peer count limit
        if self.config.peer_count_max < self.limits.peer_count.saturating_add(1) {
            debug!(
                connections = self.limits.peer_count,
                "Max peer connections limit reached"
            );
            return Err(ConnectionDenied::new(Error::MaxPeerConnectionsReached));
        }

        Ok(())
    }

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // Peer IDs checks are performed here since it is in this point where we have
        // this information.
        if self.peer_ids.is_banned(peer) {
            debug!(peer_id=%peer, "Peer is banned");
            return Err(ConnectionDenied::new(Error::BannedPeer));
        }

        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        // Dispatch pending actions.
        if let Some(action) = self.actions.pop_front() {
            return Poll::Ready(action);
        }

        let _ = self.peer_ids.poll_unpin(cx);
        let _ = self.addresses.poll_unpin(cx);

        // Perform housekeeping at regular intervals.
        if self.housekeeping_timer.poll_next_unpin(cx).is_ready() {
            self.housekeeping();
        }

        self.waker.store_waker(cx);

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::task::Context;

    use futures::{task::noop_waker, FutureExt};
    use instant::Duration;
    use libp2p::PeerId;
    use nimiq_network_interface::peer_info::Services;
    use nimiq_test_log::test;
    use nimiq_time::sleep;

    use crate::connection_pool::behaviour::ConnectionState;

    #[test(tokio::test)]
    async fn unban_peers_after_timeout() {
        let mut cs = ConnectionState::new(
            30,
            Duration::from_secs(30),
            1,
            Services::empty(),
            Duration::from_secs(2), // Ban time: 2 seconds
        );

        let waker = noop_waker();
        let cx = &mut Context::from_waker(&waker);

        let p1 = PeerId::random();
        let p2 = PeerId::random();
        let p3 = PeerId::random();

        cs.mark_banned(p1);

        sleep(Duration::from_secs(1)).await;

        cs.mark_banned(p2);
        cs.mark_banned(p3);

        sleep(Duration::from_secs(1)).await;

        // Mimic a wake
        let _ = cs.poll_unpin(cx);

        // p1 should be unbanned
        assert!(!cs.banned.contains(&p1));
        // Other banned peers should still be banned
        assert_eq!(cs.banned.len(), 2);

        sleep(Duration::from_secs(1)).await;

        // Mimic a wake
        let _ = cs.poll_unpin(cx);

        // p2 and p3 should both be unbanned
        assert!(cs.banned.is_empty());
    }
}
