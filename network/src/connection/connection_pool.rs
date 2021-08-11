use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};

use parking_lot::{ReentrantMutex, RwLock, RwLockReadGuard};

use blockchain::Blockchain;
use collections::SparseVec;
use macros::upgrade_weak;
use network_messages::SignalMessage;
use peer_address::address::PeerAddress;
use peer_address::address::{NetAddress, NetAddressType};
use peer_address::protocol::Protocol;
use utils::mutable_once::MutableOnce;
use utils::observer::PassThroughNotifier;
use utils::timers::Timers;
use utils::unique_ptr::UniquePtr;

use crate::address::peer_address_book::PeerAddressBook;
use crate::connection::{
    network_agent::{NetworkAgent, NetworkAgentEvent},
    signal_processor::SignalProcessor,
    NetworkConnection,
};
use crate::error::Error;
use crate::network_config::NetworkConfig;
#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::peer_channel::PeerChannel;
use crate::websocket::error::ConnectError;
use crate::websocket::websocket_connector::{WebSocketConnector, WebSocketConnectorEvent};
use crate::Network;
use crate::Peer;

use super::close_type::CloseType;
use super::connection_info::{ConnectionInfo, ConnectionState};

macro_rules! update_checked {
    ($peer_count: expr, $update: expr) => {
        $peer_count = match $update {
            PeerCountUpdate::Add => $peer_count + 1,
            PeerCountUpdate::Remove => $peer_count
                .checked_sub(1)
                .expect(stringify!($peer_count < 0)),
        }
    };
}

pub type ConnectionId = usize;

pub struct ConnectionPoolState {
    connections: SparseVec<ConnectionInfo>,
    connections_by_peer_address: HashMap<Arc<PeerAddress>, ConnectionId>,
    connections_by_net_address: HashMap<NetAddress, HashSet<ConnectionId>>,
    connections_by_subnet: HashMap<NetAddress, HashSet<ConnectionId>>,

    // Total bytes sent/received on past connections.
    #[cfg(feature = "metrics")]
    past_conn_metrics: NetworkMetrics,

    pub peer_count_ws: usize,
    pub peer_count_wss: usize,
    peer_count_rtc: usize,
    peer_count_dumb: usize,

    peer_count_full: usize,
    peer_count_light: usize,
    peer_count_nano: usize,

    peer_count_outbound: usize,
    peer_count_full_ws_outbound: usize,

    pub connecting_count: usize,

    inbound_count: usize,

    pub allow_inbound_connections: bool,
    pub allow_inbound_exchange: bool,

    banned_ips: HashMap<NetAddress, SystemTime>,
}

impl ConnectionPoolState {
    pub fn connection_iter(&self) -> Vec<&ConnectionInfo> {
        self.connections_by_peer_address
            .values()
            .map(|connection_id| {
                self.connections
                    .get(*connection_id)
                    .expect("Missing connection")
            })
            .collect()
    }

    pub fn id_and_connection_iter(&self) -> Vec<(ConnectionId, &ConnectionInfo)> {
        self.connections_by_peer_address
            .values()
            .map(|connection_id| {
                (
                    *connection_id,
                    self.connections
                        .get(*connection_id)
                        .expect("Missing connection"),
                )
            })
            .collect()
    }

    /// Get the connection info for a peer address.
    #[inline]
    pub fn get_connection_by_peer_address(
        &self,
        peer_address: &PeerAddress,
    ) -> Option<&ConnectionInfo> {
        Some(
            self.connections
                .get(*self.connections_by_peer_address.get(peer_address)?)
                .expect("Missing connection"),
        )
    }

    #[inline]
    pub fn get_connection_id_by_peer_address(
        &self,
        peer_address: &PeerAddress,
    ) -> Option<ConnectionId> {
        self.connections_by_peer_address.get(peer_address).cloned()
    }

    /// Get the connection info for a peer address as a mutable borrow.
    #[inline]
    pub fn get_connection_by_peer_address_mut(
        &mut self,
        peer_address: &PeerAddress,
    ) -> Option<&mut ConnectionInfo> {
        Some(
            self.connections
                .get_mut(*self.connections_by_peer_address.get(peer_address)?)
                .expect("Missing connection"),
        )
    }

    /// Get the connection info for a ConnectionId.
    #[inline]
    pub fn get_connection(&self, connection_id: ConnectionId) -> Option<&ConnectionInfo> {
        self.connections.get(connection_id)
    }

    /// Get a list of connection info for a net address.
    pub fn get_connections_by_net_address(
        &self,
        net_address: &NetAddress,
    ) -> Option<Vec<&ConnectionInfo>> {
        self.connections_by_net_address.get(net_address).map(|s| {
            s.iter()
                .map(|i| self.connections.get(*i).expect("Missing connection"))
                .collect()
        })
    }

    /// Get the number of connections for a net address.
    #[inline]
    pub fn get_num_connections_by_net_address(&self, net_address: &NetAddress) -> usize {
        self.connections_by_net_address
            .get(net_address)
            .map_or(0, HashSet::len)
    }

    /// Get a list of connection info for a subnet.
    pub fn get_connections_by_subnet(
        &self,
        net_address: &NetAddress,
    ) -> Option<Vec<&ConnectionInfo>> {
        self.connections_by_subnet
            .get(&ConnectionPool::get_subnet_address(net_address))
            .map(|s| {
                s.iter()
                    .map(|i| self.connections.get(*i).expect("Missing connection"))
                    .collect()
            })
    }

    /// Get the number of connections for a subnet.
    #[inline]
    pub fn get_num_connections_by_subnet(&self, net_address: &NetAddress) -> usize {
        self.connections_by_subnet
            .get(&ConnectionPool::get_subnet_address(net_address))
            .map_or(0, HashSet::len)
    }

    /// Retrieve a list of connection info for all outbound connections into a subnet.
    pub fn get_outbound_connections_by_subnet(
        &self,
        net_address: &NetAddress,
    ) -> Option<Vec<&ConnectionInfo>> {
        self.get_connections_by_subnet(net_address).map(|mut v| {
            v.retain(|info| {
                if let Some(network_connection) = info.network_connection() {
                    network_connection.outbound()
                } else {
                    false
                }
            });
            v
        })
    }

    /// Retrieve the number of connections for all outbound connections into a subnet.
    #[inline]
    pub fn get_num_outbound_connections_by_subnet(&self, net_address: &NetAddress) -> usize {
        self.get_outbound_connections_by_subnet(net_address)
            .map_or(0, |v| v.len())
    }

    /// Total peer count.
    #[inline]
    pub fn peer_count(&self) -> usize {
        self.peer_count_ws + self.peer_count_wss + self.peer_count_rtc + self.peer_count_dumb
    }

    /// Add a new connection to the connection pool.
    fn add(&mut self, info: ConnectionInfo) -> ConnectionId {
        let peer_address = info.peer_address();
        let connection_id = self.connections.insert(info).unwrap();

        // Add to peer address map if available.
        if let Some(peer_address) = peer_address {
            let existing_connection = self
                .connections_by_peer_address
                .insert(peer_address, connection_id);

            // Make sure we were not overwriting any previous connection id
            assert_eq!(existing_connection, None);
        }
        connection_id
    }

    /// Add a new connection to the connection pool.
    fn add_peer_address(&mut self, connection_id: ConnectionId, peer_address: Arc<PeerAddress>) {
        // Add to peer address map.
        let existing_connection = self
            .connections_by_peer_address
            .insert(peer_address, connection_id);

        // Make sure we were not overwriting any previous connection id
        assert_eq!(existing_connection, None);
    }

    /// Removes a connection from the connection pool.
    fn remove_peer_address(&mut self, connection_id: ConnectionId, peer_address: &PeerAddress) {
        // Make sure peer address does not already point to a new connection!
        if self
            .connections_by_peer_address
            .get(peer_address)
            .map(|other_connection_id| *other_connection_id == connection_id)
            .unwrap_or(false)
        {
            self.connections_by_peer_address.remove(peer_address);
        }
    }

    /// Remove a connection from the connection pool if it is present.
    fn remove(&mut self, connection_id: ConnectionId) -> Option<ConnectionInfo> {
        let info = self.connections.remove(connection_id)?;

        if let Some(peer_address) = info.peer_address() {
            self.remove_peer_address(connection_id, &peer_address);
        }

        if let Some(network_connection) = info.network_connection() {
            self.remove_net_address(connection_id, &network_connection.net_address());
        }

        Some(info)
    }

    /// Adds the net address to a connection.
    fn add_net_address(&mut self, connection_id: ConnectionId, net_address: &NetAddress) {
        // Only add reliable netAddresses.
        if !net_address.is_reliable() {
            return;
        }

        self.connections_by_net_address
            .entry(*net_address)
            .or_insert_with(HashSet::new)
            .insert(connection_id);

        let subnet_address = ConnectionPool::get_subnet_address(net_address);
        self.connections_by_subnet
            .entry(subnet_address)
            .or_insert_with(HashSet::new)
            .insert(connection_id);
    }

    /// Removes the connection from net address specific maps.
    fn remove_net_address(&mut self, connection_id: ConnectionId, net_address: &NetAddress) {
        // Only add reliable netAddresses.
        if !net_address.is_reliable() {
            return;
        }

        if let Entry::Occupied(mut occupied) = self.connections_by_net_address.entry(*net_address) {
            let is_empty = {
                let s = occupied.get_mut();

                s.remove(&connection_id);

                s.is_empty()
            };
            if is_empty {
                occupied.remove();
            }
        }

        let subnet_address = ConnectionPool::get_subnet_address(net_address);
        if let Entry::Occupied(mut occupied) = self.connections_by_subnet.entry(subnet_address) {
            let is_empty = {
                let s = occupied.get_mut();

                s.remove(&connection_id);

                s.is_empty()
            };
            if is_empty {
                occupied.remove();
            }
        }
    }

    pub fn get_peer_count_full_ws_outbound(&self) -> usize {
        self.peer_count_full_ws_outbound
    }
    pub fn get_peer_count_outbound(&self) -> usize {
        self.peer_count_outbound
    }

    pub fn count(&self) -> usize {
        self.connections_by_peer_address.len() + self.inbound_count
    }

    /// Bans an IP address.
    fn ban_ip(&mut self, net_address: &NetAddress) {
        if net_address.is_reliable() {
            warn!("Banning ip {}", net_address);
            let banned_address = if net_address.get_type() == NetAddressType::IPv4 {
                *net_address
            } else {
                net_address.subnet(64)
            };
            let unban_time = SystemTime::now() + ConnectionPool::DEFAULT_BAN_TIME;
            self.banned_ips.insert(banned_address, unban_time);
        }
    }

    /// Checks whether an IP address is banned.
    fn is_ip_banned(&self, net_address: &NetAddress) -> bool {
        !net_address.is_pseudo() && self.banned_ips.contains_key(net_address)
    }

    /// Called to regularly unban IPs.
    fn check_unban_ips(&mut self) {
        let now = SystemTime::now();
        self.banned_ips
            .retain(|_net_address, unban_time| *unban_time > now);
    }

    /// Updates the number of connected peers.
    fn update_connected_peer_count(&mut self, connection: Connection, update: PeerCountUpdate) {
        // We assume the connection to be present and having a valid peer address/network connection.
        let info = match connection {
            Connection::Id(connection_id) => self.connections.get(connection_id).unwrap(),
            Connection::Info(info) => info,
        };
        let peer_address = info.peer_address().unwrap();
        let network_connection = info.network_connection().unwrap();

        match peer_address.protocol() {
            Protocol::Wss => update_checked!(self.peer_count_wss, update),
            Protocol::Ws => update_checked!(self.peer_count_ws, update),
            Protocol::Rtc => update_checked!(self.peer_count_rtc, update),
            Protocol::Dumb => update_checked!(self.peer_count_dumb, update),
        }

        if peer_address.services.is_full_node() {
            update_checked!(self.peer_count_full, update);
        } else if peer_address.services.is_light_node() {
            update_checked!(self.peer_count_light, update);
        } else if peer_address.services.is_nano_node() {
            update_checked!(self.peer_count_nano, update);
        }

        if network_connection.outbound() {
            update_checked!(self.peer_count_outbound, update);
            if peer_address.services.is_full_node()
                && (peer_address.protocol() == Protocol::Wss
                    || peer_address.protocol() == Protocol::Ws)
            {
                update_checked!(self.peer_count_full_ws_outbound, update);
            }
        }
    }

    #[cfg(feature = "metrics")]
    pub fn get_past_conn_metrics(&self) -> &NetworkMetrics {
        &self.past_conn_metrics
    }

    #[cfg(feature = "metrics")]
    // Update the metrics on past connections when a peer leaves.
    fn update_past_conn_metrics(&mut self, metrics: &NetworkMetrics) {
        self.past_conn_metrics
            .note_bytes_received(metrics.bytes_received());

        self.past_conn_metrics.note_bytes_sent(metrics.bytes_sent());
    }
}

enum Connection<'a> {
    Id(ConnectionId),
    Info(&'a ConnectionInfo),
}

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy)]
enum ConnectionPoolTimer {
    UnbanIps,
}

pub struct ConnectionPool {
    blockchain: Arc<Blockchain>,
    network_config: Arc<NetworkConfig>,
    addresses: Arc<PeerAddressBook>,

    websocket_connector: WebSocketConnector,

    signal_processor: SignalProcessor,

    state: RwLock<ConnectionPoolState>,
    change_lock: ReentrantMutex<()>,

    pub notifier: RwLock<PassThroughNotifier<'static, ConnectionPoolEvent>>,
    timers: Timers<ConnectionPoolTimer>,
    self_weak: MutableOnce<Weak<ConnectionPool>>,
}

impl ConnectionPool {
    pub const PEER_COUNT_MAX: usize = 4000;
    const PEER_COUNT_PER_IP_MAX: usize = 20;
    const PEER_COUNT_DUMB_MAX: usize = 1000;
    const OUTBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 2;
    const INBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 100;
    const IPV4_SUBNET_MASK: u8 = 24;
    const IPV6_SUBNET_MASK: u8 = 96;

    const DEFAULT_BAN_TIME: Duration = Duration::from_secs(60 * 10);
    // seconds
    const UNBAN_IPS_INTERVAL: Duration = Duration::from_secs(60); // seconds

    /// Constructor.
    pub fn new(
        peer_address_book: Arc<PeerAddressBook>,
        network_config: Arc<NetworkConfig>,
        blockchain: Arc<Blockchain>,
    ) -> Result<Arc<Self>, Error> {
        if !network_config.is_initialized() {
            return Err(Error::UninitializedPeerKey);
        }

        let pool = Arc::new(Self {
            blockchain,
            network_config: network_config.clone(),
            addresses: peer_address_book.clone(),

            websocket_connector: WebSocketConnector::new(network_config.clone()),

            signal_processor: SignalProcessor::new(peer_address_book, network_config),

            state: RwLock::new(ConnectionPoolState {
                connections: SparseVec::new(),
                connections_by_peer_address: HashMap::new(),
                connections_by_net_address: HashMap::new(),
                connections_by_subnet: HashMap::new(),

                #[cfg(feature = "metrics")]
                past_conn_metrics: NetworkMetrics::default(),

                peer_count_ws: 0,
                peer_count_wss: 0,
                peer_count_rtc: 0,
                peer_count_dumb: 0,

                peer_count_full: 0,
                peer_count_light: 0,
                peer_count_nano: 0,

                peer_count_outbound: 0,
                peer_count_full_ws_outbound: 0,

                connecting_count: 0,

                inbound_count: 0,

                allow_inbound_connections: false,
                allow_inbound_exchange: false,

                banned_ips: HashMap::new(),
            }),
            change_lock: ReentrantMutex::new(()),

            notifier: RwLock::new(PassThroughNotifier::new()),
            timers: Timers::new(),
            self_weak: MutableOnce::new(Weak::new()),
        });
        // Initialise.
        {
            unsafe { pool.self_weak.replace(Arc::downgrade(&pool)) };
            let weak = pool.self_weak.clone();
            pool.websocket_connector
                .notifier
                .write()
                .register(move |event| {
                    let pool = upgrade_weak!(weak);
                    match event {
                        WebSocketConnectorEvent::Connection(conn) => {
                            pool.on_connection(conn);
                        }
                        WebSocketConnectorEvent::Error(peer_address, error) => {
                            pool.on_connect_error(peer_address, error);
                        }
                    }
                });
        }
        Ok(pool)
    }

    /// Initialises necessary threads.
    pub fn initialize(&self) -> Result<(), Error> {
        // Start accepting incoming connections.
        self.websocket_connector.start()?;

        let weak = self.self_weak.clone();
        self.timers.set_interval(
            ConnectionPoolTimer::UnbanIps,
            move || {
                let this = upgrade_weak!(weak);
                this.state.write().check_unban_ips();
            },
            Self::UNBAN_IPS_INTERVAL,
        );
        Ok(())
    }

    /// Initiates a outbound connection.
    pub fn connect_outbound(&self, peer_address: Arc<PeerAddress>) -> bool {
        let _guard = self.change_lock.lock();

        // All checks in one step.
        if !self.check_outbound_connection_request(peer_address.clone()) {
            return false;
        }

        // Connection request accepted.

        // Choose connector type and call.
        let handle = match self.websocket_connector.connect(peer_address.clone()) {
            Ok(handle) => handle,
            Err(e) => {
                warn!(
                    "Could not connect outbound to {}, error: {}",
                    peer_address, e
                );
                return false;
            }
        };

        // Create fresh ConnectionInfo instance.
        let mut state = self.state.write();
        let connection_id = state.add(ConnectionInfo::outbound(peer_address));
        state
            .connections
            .get_mut(connection_id)
            .unwrap()
            .set_connection_handle(handle);

        state.connecting_count += 1;

        true
    }

    pub fn disconnect(&self) {
        let state = self.state.read();
        for connection in state.connection_iter() {
            if let Some(peer_channel) = connection.peer_channel() {
                peer_channel.close(CloseType::ManualNetworkDisconnect);
            }
        }
    }

    /// Returns a mapped RwLockReadGuard for the internal state.
    pub fn state(&self) -> RwLockReadGuard<ConnectionPoolState> {
        self.state.read()
    }

    /// Close a connection.
    fn close(network_connection: Option<&NetworkConnection>, ty: CloseType) {
        if let Some(network_connection) = network_connection {
            network_connection.close(ty);
        }
    }

    /// Checks the validity of a connection from `on_connection`.
    fn check_connection(state: &ConnectionPoolState, connection_id: ConnectionId) -> bool {
        let info = state.connections.get(connection_id).unwrap();
        let conn = info.network_connection();
        assert!(conn.is_some(), "Connection must be established");
        let conn = conn.unwrap();

        // Close connection if we currently do not allow inbound connections.
        // TODO WebRTC connections are exempt.
        if conn.inbound() && !state.allow_inbound_connections {
            Self::close(
                info.network_connection(),
                CloseType::InboundConnectionsBlocked,
            );
            return false;
        }

        let net_address = conn.net_address();
        if net_address.is_reliable() {
            // Close connection if peer's IP is banned.
            if state.is_ip_banned(&net_address) {
                Self::close(info.network_connection(), CloseType::BannedIp);
                return false;
            }

            // Close connection if we have too many connections to the peer's IP address.
            if state.get_num_connections_by_net_address(&net_address) > Self::PEER_COUNT_PER_IP_MAX
            {
                Self::close(info.network_connection(), CloseType::ConnectionLimitPerIp);
                return false;
            }

            // Close connection if we have too many connections to the peer's subnet.
            if state.get_num_connections_by_subnet(&net_address)
                > Self::INBOUND_PEER_COUNT_PER_SUBNET_MAX
            {
                Self::close(info.network_connection(), CloseType::ConnectionLimitPerIp);
                return false;
            }
        }

        // Reject peer if we have reached max peer count.
        // There are two exceptions to this: outbound connections
        // and inbound connections with inbound exchange set.
        if state.peer_count() >= Self::PEER_COUNT_MAX
            && !conn.outbound()
            && !(conn.inbound() && state.allow_inbound_exchange)
        {
            Self::close(info.network_connection(), CloseType::MaxPeerCountReached);
            return false;
        }
        true
    }

    /// Callback upon connection establishment.
    fn on_connection(&self, connection: NetworkConnection) {
        let guard = self.change_lock.lock();

        let agent;
        let connection_id;
        // Acquire write lock and release it again before notifying listeners.
        {
            let mut state = self.state.write();
            if connection.outbound() {
                let peer_address = connection
                    .peer_address()
                    .expect("Outbound connection without peer address");
                let connection_id_opt = state.connections_by_peer_address.get(&peer_address);

                if connection_id_opt.is_none() {
                    Self::close(Some(&connection), CloseType::InvalidConnectionState);
                    error!(
                        "No ConnectionInfo present for outgoing connection ({})",
                        peer_address
                    );
                    return;
                }

                connection_id = *connection_id_opt.unwrap();
                if state.connections.get(connection_id).unwrap().state()
                    != ConnectionState::Connecting
                {
                    Self::close(Some(&connection), CloseType::InvalidConnectionState);
                    error!("Expected state to be connecting ({})", peer_address);
                    return;
                }

                update_checked!(state.connecting_count, PeerCountUpdate::Remove);

                // Set peerConnection to CONNECTED state.
                state
                    .connections
                    .get_mut(connection_id)
                    .unwrap()
                    .set_network_connection(connection);
            } else {
                // Add connection (without having obtained peer address).
                connection_id = state.add(ConnectionInfo::inbound(connection));
                state.inbound_count += 1;
            }

            // Register close listener early to clean up correctly in case _checkConnection() closes the connection.
            let info = state
                .connections
                .get(connection_id)
                .unwrap_or_else(|| panic!("Missing connection #{}", connection_id));

            // There is a chance that there was a race condition in the websocket connector
            // and that the connection should actually have been aborted.
            if info
                .connection_handle()
                .map(|handle| handle.is_aborted())
                .unwrap_or(false)
            {
                Self::close(info.network_connection(), CloseType::SimultaneousConnection);
                debug!("Connection should have been aborted in connecting state, closing it now");
                return;
            }

            let peer_channel = Arc::new(PeerChannel::new(info.network_connection().unwrap()));
            let weak = self.self_weak.clone();
            peer_channel
                .close_notifier
                .write()
                .register(move |ty: &CloseType| {
                    let arc = upgrade_weak!(weak);
                    arc.on_close(connection_id, *ty);
                });

            if !Self::check_connection(&state, connection_id) {
                return;
            }

            // Connection accepted.

            let net_address = info
                .network_connection()
                .map(NetworkConnection::net_address);

            if let Some(ref net_address) = net_address {
                state.add_net_address(connection_id, net_address);
            }

            // The extra lookup is needed to satisfy the borrow checker.
            let info = state
                .connections
                .get_mut(connection_id)
                .unwrap_or_else(|| panic!("Missing connection #{}", connection_id));
            info.drop_connection_handle();

            let conn_type = if info.network_connection().unwrap().inbound() {
                "inbound"
            } else {
                "outbound"
            };
            debug!(
                "Connection established ({}) #{} {} {}",
                conn_type,
                connection_id,
                net_address.map_or("<unknown>".to_string(), |n| n.to_string()),
                info.peer_address()
                    .map_or("<unknown>".to_string(), |p| p.to_string())
            );

            // Set the peer_channel.
            info.set_peer_channel(peer_channel.clone());

            // Create NetworkAgent.
            agent = NetworkAgent::new(
                Arc::clone(&self.blockchain),
                self.addresses.clone(),
                self.network_config.clone(),
                peer_channel,
            );
            let mut locked_agent = agent.write();
            let weak = self.self_weak.clone();
            locked_agent
                .notifier
                .register(move |event: &NetworkAgentEvent| {
                    let pool = upgrade_weak!(weak);
                    match event {
                        NetworkAgentEvent::Version(ref peer) => {
                            pool.check_handshake(connection_id, peer)
                        }
                        NetworkAgentEvent::Handshake(ref peer) => {
                            pool.on_handshake(connection_id, peer)
                        }
                        _ => {}
                    }
                });

            info.set_network_agent(agent.clone());
        }

        // Drop the guard before notifying.
        drop(guard);

        // Let listeners know about this connection.
        self.notifier
            .read()
            .notify(ConnectionPoolEvent::Connection(connection_id));

        // Initiate handshake with the peer.
        agent.write().handshake();
    }

    /// Checks the validity of a handshake.
    /// Is called after a version message.
    fn check_handshake(&self, connection_id: ConnectionId, peer: &UniquePtr<Peer>) {
        let _guard = self.change_lock.lock();

        // Read lock.
        {
            let state = self.state.read();
            let info = state
                .get_connection(connection_id)
                .unwrap_or_else(|| panic!("Missing connection #{}", connection_id));

            // Close connection if peer's address is banned.
            let peer_address = peer.peer_address();
            if self.addresses.is_banned(&peer_address) {
                Self::close(info.network_connection(), CloseType::PeerIsBanned);
                return;
            }

            // Duplicate/simultaneous connection check (post version):
            let stored_connection_id = state.connections_by_peer_address.get(&peer_address);
            if let Some(stored_connection_id) = stored_connection_id {
                if *stored_connection_id != connection_id {
                    // If we already have an established connection to this peer, close this connection.
                    let stored_connection = state
                        .connections
                        .get(*stored_connection_id)
                        .unwrap_or_else(|| panic!("Missing connection #{}", *stored_connection_id));
                    if stored_connection.state() == ConnectionState::Established {
                        Self::close(info.network_connection(), CloseType::DuplicateConnection);
                        return;
                    }
                }
            }

            // Close connection if we have too many dumb connections.
            if peer_address.protocol() == Protocol::Dumb
                && state.peer_count_dumb >= Self::PEER_COUNT_DUMB_MAX
            {
                Self::close(info.network_connection(), CloseType::ConnectionLimitDumb);
                return;
            }
        }

        // Set peerConnection to NEGOTIATING state.
        self.state
            .write()
            .connections
            .get_mut(connection_id)
            .unwrap()
            .negotiating();
    }

    /// Callback during handshake.
    fn on_handshake(&self, connection_id: ConnectionId, peer: &UniquePtr<Peer>) {
        let guard = self.change_lock.lock();

        let peer_address = peer.peer_address();
        let mut is_inbound = false;
        // Read lock.
        {
            let mut state = self.state.write();
            if let Some(info) = state.connections.get(connection_id) {
                let network_connection = info.network_connection().unwrap();

                if network_connection.inbound() {
                    // Re-check allowInboundExchange as it might have changed.
                    if state.peer_count() >= Self::PEER_COUNT_MAX && !state.allow_inbound_exchange {
                        Self::close(info.network_connection(), CloseType::MaxPeerCountReached);
                        return;
                    }

                    // Duplicate/simultaneous connection check (post handshake):
                    let stored_connection_id = state.connections_by_peer_address.get(&peer_address);
                    if let Some(&stored_connection_id) = stored_connection_id {
                        if stored_connection_id != connection_id {
                            let stored_connection = state
                                .connections
                                .get(stored_connection_id)
                                .unwrap_or_else(|| {
                                    panic!("Missing connection #{}", stored_connection_id)
                                });
                            match stored_connection.state() {
                                ConnectionState::Connecting => {
                                    // Abort the stored connection attempt and accept this connection.
                                    let protocol = peer_address.protocol();
                                    assert!(
                                        protocol == Protocol::Wss || protocol == Protocol::Ws,
                                        "Duplicate connection to non-WS node"
                                    );
                                    debug!("Aborting connection attempt to {}, simultaneous connection succeeded", peer_address);

                                    // Abort connection.
                                    if let Some(handle) = stored_connection.connection_handle() {
                                        handle.abort(CloseType::SimultaneousConnection);
                                    }

                                    // Clean up the state from the changes made by connect_outbound()
                                    state.connections.remove(stored_connection_id);
                                    state.remove_peer_address(stored_connection_id, &peer_address);
                                }

                                ConnectionState::Established => {
                                    // If we have another established connection to this peer, close this connection.
                                    Self::close(
                                        info.network_connection(),
                                        CloseType::SimultaneousConnection,
                                    );
                                    return;
                                }
                                ConnectionState::Negotiating => {
                                    // The peer with the lower peerId accepts this connection and closes his stored connection.
                                    if self.network_config.peer_id() < peer_address.peer_id() {
                                        Self::close(
                                            stored_connection.network_connection(),
                                            CloseType::SimultaneousConnection,
                                        );
                                        // Free association with peer address.
                                        state.remove_peer_address(
                                            stored_connection_id,
                                            &peer_address,
                                        );
                                    } else {
                                        // The peer with the higher peerId closes this connection and keeps his stored connection.
                                        Self::close(
                                            info.network_connection(),
                                            CloseType::SimultaneousConnection,
                                        );
                                        return;
                                    }
                                }
                                _ => {
                                    // Accept this connection and close the stored connection.
                                    Self::close(
                                        stored_connection.network_connection(),
                                        CloseType::SimultaneousConnection,
                                    );
                                    // Free association with peer address.
                                    state.remove_peer_address(stored_connection_id, &peer_address);
                                }
                            }
                        }
                    }

                    is_inbound = true;
                }
            } else {
                warn!("Missing connection #{}", connection_id);
                return;
            }
        }

        // Write lock.
        if is_inbound {
            let mut state = self.state.write();
            assert!(
                state
                    .get_connection_by_peer_address(&peer_address)
                    .is_none(),
                "ConnectionInfo already exists"
            );
            state
                .connections
                .get_mut(connection_id)
                .unwrap()
                .set_peer_address(peer_address.clone());
            state.add_peer_address(connection_id, peer_address.clone());

            update_checked!(state.inbound_count, PeerCountUpdate::Remove);
        }

        // Handshake accepted.

        // Check if we need to recycle a connection.
        if self.peer_count() >= Self::PEER_COUNT_MAX {
            // This will most likely lead to reentering the guard.
            self.notifier
                .read()
                .notify(ConnectionPoolEvent::RecyclingRequest);
        }

        // Acquire write lock and release it again before notifying listeners.
        {
            let mut state = self.state.write();
            // Set ConnectionInfo to Established state.
            state
                .connections
                .get_mut(connection_id)
                .unwrap()
                .set_peer(peer.as_ref().clone()); // TODO do we need a clone here?

            if let Some(net_address) = peer.net_address() {
                // The HashSet takes care of only inserting it once.
                state.add_net_address(connection_id, &net_address);
            }

            state.update_connected_peer_count(Connection::Id(connection_id), PeerCountUpdate::Add);
        }

        let state = self.state.read();
        let info = state
            .get_connection(connection_id)
            .unwrap_or_else(|| panic!("Missing connection #{}", connection_id));

        // Setup signal forwarding.
        if Network::SIGNALING_ENABLED {
            let self_weak = self.self_weak.clone();
            let weak_peer_channel =
                Arc::downgrade(&info.peer_channel().expect("Missing peer channel"));
            peer.channel
                .msg_notifier
                .signal
                .write()
                .register(move |msg: SignalMessage| {
                    let this = upgrade_weak!(self_weak);
                    let peer_channel = upgrade_weak!(weak_peer_channel);
                    this.signal_processor.on_signal(peer_channel, msg);
                });
        }

        // Mark address as established.
        self.addresses
            .established(info.peer_channel().unwrap(), peer_address.clone());

        // Drop the locks before notifying.
        drop(state);
        drop(guard);

        debug!(
            "Peer joined: {} (v{}, {:?}, {})",
            &peer_address,
            peer.version,
            peer_address.services,
            peer.user_agent.as_ref().unwrap_or(&"None".to_string())
        );

        // Let listeners know about this peer.
        self.notifier
            .read()
            .notify(ConnectionPoolEvent::PeerJoined(peer.as_ref().clone()));

        // Let listeners know that the peers changed.
        self.notifier
            .read()
            .notify(ConnectionPoolEvent::PeersChanged);
    }

    /// Callback upon closing of connection.
    fn on_close(&self, connection_id: ConnectionId, ty: CloseType) {
        let mut established_peer_left = false;
        let mut info;
        {
            let guard = self.change_lock.lock();

            // Only propagate the close type (i.e. track fails/bans) if the peerAddress is set.
            // This is true for
            // - all outbound connections
            // - inbound connections post handshake (peerAddress is verified)
            {
                let state = self.state.read();
                let info = state
                    .get_connection(connection_id)
                    .unwrap_or_else(|| panic!("Missing connection #{}", connection_id));
                if let Some(peer_address) = info.peer_address() {
                    self.addresses.close(info.peer_channel(), peer_address, ty);
                }
            }

            // Acquire write lock and release it again before notifying listeners.
            {
                let mut state = self.state.write();
                info = state
                    .remove(connection_id)
                    .unwrap_or_else(|| panic!("Missing connection #{}", connection_id));

                // This unwrap will always succeed because the handler we're in (on_close) is setup after the network_connection
                // has already been set (in the on_connection handler) and before the network_connection is removed with info.close().
                #[cfg(feature = "metrics")]
                state.update_past_conn_metrics(info.network_connection().unwrap().metrics());

                // Check if the handshake with this peer has completed.
                if info.state() == ConnectionState::Established {
                    let net_address = info
                        .network_connection()
                        .map(NetworkConnection::net_address);
                    // If closing is due to a ban, also ban the IP
                    if ty.is_banning_type() {
                        if let Some(ref net_address) = net_address {
                            state.ban_ip(net_address);
                        }
                    }

                    state.update_connected_peer_count(
                        Connection::Info(&info),
                        PeerCountUpdate::Remove,
                    );

                    established_peer_left = true;

                    debug!(
                        "Peer left: {} {} (version={:?}, closeType={:?})",
                        info.peer_address().unwrap(),
                        net_address.unwrap(),
                        info.peer().map(|p| p.version),
                        ty
                    );
                } else {
                    match info.network_connection().map(NetworkConnection::inbound) {
                        Some(true) => {
                            state
                                .inbound_count
                                .checked_sub(1)
                                .expect("inbound_count < 0");
                            debug!(
                                "Inbound connection #{} closed pre-handshake: {:?}",
                                connection_id, ty
                            );
                        }
                        Some(false) => {
                            // Drop guard/lock before notifying (all changes done).
                            drop(state);
                            drop(guard);

                            debug!(
                                "Connection #{} to {} closed pre-handshake: {:?}",
                                connection_id,
                                info.peer_address().unwrap(),
                                ty
                            );
                            // Only sets a timer and won't reenter connection pool.
                            self.notifier
                                .read()
                                .notify(ConnectionPoolEvent::ConnectError(
                                    info.peer_address().expect("PeerAddress not set"),
                                    ty,
                                ));
                        }
                        _ => unreachable!(format!(
                            "Invalid state, closing connection #{} with network connection not set",
                            connection_id
                        )),
                    }
                }
            }
        } // Implicitly drop guard through scoping.

        if established_peer_left {
            // Tell listeners that this peer has gone away.
            self.notifier.read().notify(ConnectionPoolEvent::PeerLeft(
                info.peer().expect("Peer not set").clone(),
            ));

            // Let listeners know that the peers changed.
            self.notifier
                .read()
                .notify(ConnectionPoolEvent::PeersChanged);
        }

        // Set the peer connection to closed state.
        info.close();
    }

    /// Total peer count.
    pub fn peer_count(&self) -> usize {
        let state = self.state.read();
        state.peer_count()
    }

    /// Connecting count.
    pub fn connecting_count(&self) -> usize {
        let state = self.state.read();
        state.connecting_count
    }

    pub fn count(&self) -> usize {
        let state = self.state.read();
        state.count()
    }

    pub fn peer_count_outbound(&self) -> usize {
        self.state.read().peer_count_outbound
    }

    pub fn allow_inbound_exchange(&self) -> bool {
        self.state.read().allow_inbound_exchange
    }
    pub fn allow_inbound_connections(&self) -> bool {
        self.state.read().allow_inbound_connections
    }

    pub fn set_allow_inbound_exchange(&self, allow_inbound_exchange: bool) {
        let _guard = self.change_lock.lock();
        self.state.write().allow_inbound_exchange = allow_inbound_exchange;
    }
    pub fn set_allow_inbound_connections(&self, allow_inbound_connections: bool) {
        let _guard = self.change_lock.lock();
        self.state.write().allow_inbound_connections = allow_inbound_connections;
    }

    /// Callback on connect error.
    fn on_connect_error(&self, peer_address: Arc<PeerAddress>, error: ConnectError) {
        let guard = self.change_lock.lock();
        debug!("Connection to {} failed with error {}", peer_address, error);

        // Acquire write lock and release it again before notifying listeners.
        {
            let mut state = self.state.write();
            if let Some(&connection_id) = state.connections_by_peer_address.get(&peer_address) {
                let info = state
                    .connections
                    .get(connection_id)
                    .unwrap_or_else(|| panic!("Missing connection #{}", connection_id));
                assert_eq!(
                    info.state(),
                    ConnectionState::Connecting,
                    "ConnectionInfo state not Connecting, but {:?} ({})",
                    info.state(),
                    peer_address
                );
                state.remove(connection_id).unwrap();
            }

            update_checked!(state.connecting_count, PeerCountUpdate::Remove);
        }

        self.addresses
            .close(None, peer_address.clone(), CloseType::ConnectionFailed);

        // Drop the guard before notifying.
        drop(guard);

        // Notify about error if it was not aborted by us due to a simultaneous connection
        match error {
            ConnectError::AbortedByUs => (),
            _ => self
                .notifier
                .read()
                .notify(ConnectionPoolEvent::ConnectError(
                    peer_address,
                    CloseType::ConnectionFailed,
                )),
        }
    }

    /// Convert a net address into a subnet according to the configured bitmask.
    fn get_subnet_address(net_address: &NetAddress) -> NetAddress {
        let bit_mask = if net_address.get_type() == NetAddressType::IPv4 {
            Self::IPV4_SUBNET_MASK
        } else {
            Self::IPV6_SUBNET_MASK
        };
        net_address.subnet(bit_mask)
    }

    /// Check the validity of a outbound connection request (e.g. no duplicate connections).
    fn check_outbound_connection_request(&self, peer_address: Arc<PeerAddress>) -> bool {
        match peer_address.protocol() {
            Protocol::Wss => {}
            Protocol::Ws => {}
            _ => {
                error!("Cannot connect to {} - unsupported protocol", peer_address);
                return false;
            }
        }

        if self.addresses.is_banned(&peer_address) {
            error!("Connecting to banned address {}", peer_address);
            return false;
        }

        let state = self.state.read();
        let info = state.get_connection_by_peer_address(&peer_address);
        if info.is_some() {
            error!("Duplicate connection to {}", peer_address);
            return false;
        }

        // Forbid connection if we have too many connections to the peer's IP address.
        if peer_address.net_address.is_reliable() {
            if state.get_num_connections_by_net_address(&peer_address.net_address)
                >= Self::PEER_COUNT_PER_IP_MAX
            {
                warn!(
                    "Connection limit per IP ({}) reached ({})",
                    Self::PEER_COUNT_PER_IP_MAX,
                    peer_address.net_address
                );
                return false;
            }

            if state.get_num_outbound_connections_by_subnet(&peer_address.net_address)
                >= Self::OUTBOUND_PEER_COUNT_PER_SUBNET_MAX
            {
                warn!(
                    "Connection limit per IP ({}) reached ({})",
                    Self::OUTBOUND_PEER_COUNT_PER_SUBNET_MAX,
                    peer_address.net_address
                );
                return false;
            }
        }

        true
    }
}

enum PeerCountUpdate {
    Add,
    Remove,
}

pub enum ConnectionPoolEvent {
    PeerJoined(Peer),
    PeerLeft(Peer),
    PeersChanged,
    ConnectError(Arc<PeerAddress>, CloseType),
    Connection(ConnectionId),
    RecyclingRequest,
}
