use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio;

use crate::network;
use crate::network::address::net_address::{NetAddress, NetAddressType};
use crate::network::address::peer_address::PeerAddress;
use crate::network::connection::NetworkConnection;
use crate::network::Protocol;

use super::close_type::CloseType;
use super::connection_info::{ConnectionInfo, ConnectionState};
use std::collections::HashSet;
use std::collections::LinkedList;
use crate::network::Peer;
use crate::network::address::peer_address_book::PeerAddressBook;
use crate::utils::observer::{Notifier, weak_listener};
use crate::network::peer_channel::PeerChannel;
use std::sync::Weak;
use parking_lot::RwLock;
use crate::network::peer_channel::PeerChannelEvent;

macro_rules! update_checked {
    ($peer_count: expr, $update: expr) => {
        $peer_count = match $update {
            PeerCountUpdate::Add => $peer_count + 1,
            PeerCountUpdate::Remove => $peer_count.checked_sub(1).expect(stringify!($peer_count < 0)),
        }
    };
}

type ConnectionId = usize;


pub struct ConnectionPool<'network> {
    connections: SparseVec<ConnectionInfo<'network>>,
    connections_by_peer_address: HashMap<Arc<PeerAddress>, ConnectionId>,
    connections_by_net_address: HashMap<NetAddress, HashSet<ConnectionId>>,
    connections_by_subnet: HashMap<NetAddress, HashSet<ConnectionId>>,

    peer_count_ws: usize,
    peer_count_wss: usize,
    peer_count_rtc: usize,
    peer_count_dumb: usize,

    peer_count_full: usize,
    peer_count_light: usize,
    peer_count_nano: usize,

    peer_count_outbound: usize,
    peer_count_full_ws_outbound: usize,

    connecting_count: usize,

    inbound_count: usize,

    allow_inbound_connections: bool,
    allow_inbound_exchange: bool,

    banned_ips: HashMap<NetAddress, SystemTime>,

    addresses: PeerAddressBook<'network>,

    notifier: Notifier<'network, ConnectionPoolEvent<'network>>,

    listener: Weak<RwLock<ConnectionPool<'network>>>,
}

impl<'network> ConnectionPool<'network> {
    const DEFAULT_BAN_TIME: Duration = Duration::from_secs(60 * 10); // seconds

    /// Constructor.
    pub fn new(peer_address_book: PeerAddressBook<'network>) -> Arc<RwLock<Self>> {
        let arc = Arc::new(RwLock::new(ConnectionPool {
            connections: SparseVec::new(),
            connections_by_peer_address: HashMap::new(),
            connections_by_net_address: HashMap::new(),
            connections_by_subnet: HashMap::new(),

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

            addresses: peer_address_book,

            notifier: Notifier::new(),

            listener: Weak::new(),
        }));
        arc.write().listener = Arc::downgrade(&arc);
        arc
    }

    /// Initiates a outbound connection.
    pub fn connect_outbound(&mut self, peer_address: Arc<PeerAddress>) -> bool {
        // All checks in one step.
        if !self.check_outbound_connection_request(peer_address.clone()) {
            return false;
        }

        // Connection request accepted.

        // Create fresh ConnectionInfo instance.
        let connection_id = self.add(ConnectionInfo::outbound(peer_address.clone()));

        // Choose connector type and call.
        let mut connecting = false;
        match peer_address.protocol() {
            Protocol::Wss => {
                // TODO
                connecting = true;
            },
            Protocol::Ws => {
                // TODO
                connecting = true;
            },
            _ => {
                unreachable!("Cannot connect to non-WS(S) nodes.");
            },
        }

        if connecting {
            self.connecting_count += 1;
        } else {
            self.remove(connection_id);
            debug!("Outbound attempt not connecting: {:?}", peer_address);
            return false;
        }

        return true;
    }

    /// Get the connection info for a peer address.
    pub fn get_connection_by_peer_address(&self, peer_address: &PeerAddress) -> Option<&ConnectionInfo<'network>> {
        Some(self.connections.get(*self.connections_by_peer_address.get(peer_address)?).expect("Missing connection"))
    }

    /// Get the connection info for a peer address as a mutable borrow.
    pub fn get_connection_by_peer_address_mut(&mut self, peer_address: &PeerAddress) -> Option<&mut ConnectionInfo<'network>> {
        Some(self.connections.get_mut(*self.connections_by_peer_address.get(peer_address)?).expect("Missing connection"))
    }

    /// Get the connection info for a ConnectionId.
    pub fn get_connection(&mut self, connection_id: ConnectionId) -> Option<&ConnectionInfo<'network>> {
        self.connections.get(connection_id)
    }

    /// Get a list of connection info for a net address.
    pub fn get_connections_by_net_address(&self, net_address: &NetAddress) -> Option<Vec<&ConnectionInfo<'network>>> {
        self.connections_by_net_address.get(net_address).map(|s| {
            s.iter().map(|i| self.connections.get(*i).expect("Missing connection")).collect()
        })
    }

    /// Get the number of connections for a net address.
    pub fn get_num_connections_by_net_address(&self, net_address: &NetAddress) -> usize {
        self.connections_by_net_address.get(net_address).map_or(0, |s| s.len())
    }

    /// Get a list of connection info for a subnet.
    pub fn get_connections_by_subnet(&self, net_address: &NetAddress) -> Option<Vec<&ConnectionInfo<'network>>> {
        self.connections_by_subnet.get(&ConnectionPool::get_subnet_address(net_address)).map(|s| {
            s.iter().map(|i| self.connections.get(*i).expect("Missing connection")).collect()
        })
    }

    /// Get the number of connections for a subnet.
    pub fn get_num_connections_by_subnet(&self, net_address: &NetAddress) -> usize {
        self.connections_by_subnet.get(&ConnectionPool::get_subnet_address(net_address)).map_or(0, |s| s.len())
    }

    /// Retrieve a list of connection info for all outbound connections into a subnet.
    pub fn get_outbound_connections_by_subnet(&self, net_address: &NetAddress) -> Option<Vec<&ConnectionInfo<'network>>> {
        self.get_connections_by_subnet(net_address)
            .map(|mut v| {
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
    pub fn get_num_outbound_connections_by_subnet(&self, net_address: &NetAddress) -> usize {
        self.get_outbound_connections_by_subnet(net_address).map_or(0, |v| v.len())
    }

    /// Close a connection.
    fn close(network_connection: Option<&NetworkConnection>, ty: CloseType) {
        if let Some(network_connection) = network_connection {
            tokio::spawn(network_connection.close(ty));
        }
    }

    /// Checks the validity of a connection.
    fn check_connection(&self, connection_id: ConnectionId) -> bool {
        let info = self.connections.get(connection_id).unwrap();
        let conn = info.network_connection();
        assert!(conn.is_some(), "Connection must be established");
        let conn = conn.unwrap();

        // Close connection if we currently do not allow inbound connections.
        // TODO WebRTC connections are exempt.
        if conn.inbound() && !self.allow_inbound_connections {
            ConnectionPool::close(info.network_connection(), CloseType::InboundConnectionsBlocked);
            return false;
        }

        let net_address = conn.net_address();
        if net_address.is_reliable() {
            // Close connection if peer's IP is banned.
            if self.is_ip_banned(&net_address) {
                ConnectionPool::close(info.network_connection(), CloseType::BannedIp);
                return false;
            }

            // Close connection if we have too many connections to the peer's IP address.
            if self.get_num_connections_by_net_address(&net_address) > network::PEER_COUNT_PER_IP_MAX {
                ConnectionPool::close(info.network_connection(), CloseType::ConnectionLimitPerIp);
                return false;
            }

            // Close connection if we have too many connections to the peer's subnet.
            if self.get_num_connections_by_subnet(&net_address) > network::INBOUND_PEER_COUNT_PER_SUBNET_MAX {
                ConnectionPool::close(info.network_connection(), CloseType::ConnectionLimitPerIp);
                return false;
            }
        }

        // Reject peer if we have reached max peer count.
        // There are two exceptions to this: outbound connections
        // and inbound connections with inbound exchange set.
        if self.peer_count() >= network::PEER_COUNT_MAX
            && !conn.outbound()
            && !(conn.inbound() && self.allow_inbound_exchange) {

            ConnectionPool::close(info.network_connection(), CloseType::MaxPeerCountReached);
            return false;
        }
        return true;
    }

    fn on_peer_channel_event(&mut self, connection_id: ConnectionId, event: PeerChannelEvent) {
        match event {
            PeerChannelEvent::Close(ty) => self.on_close(connection_id, ty),
//             TODO PeerChannelEvent::Error => self.on_connect_error(),
            _ => {},
        }
    }

    /// Callback upon connection establishment.
    fn on_connection(&mut self, connection: NetworkConnection<'network>) {
        let connection_id;
        if connection.outbound() {
            let peer_address = connection.peer_address().expect("Outbound connection without peer address");
            let connection_id_opt = self.connections_by_peer_address.get(&peer_address);

            if connection_id_opt.is_none() {
                ConnectionPool::close(Some(&connection), CloseType::InvalidConnectionState);
                error!("No ConnectionInfo present for outgoing connection ({:?}", peer_address);
                return;
            }

            connection_id = *connection_id_opt.unwrap();
            if self.connections.get(connection_id).unwrap().state() != ConnectionState::Connecting {
                ConnectionPool::close(Some(&connection), CloseType::InvalidConnectionState);
                error!("Expected state to be connecting ({:?}", peer_address);
                return;
            }

            update_checked!(self.connecting_count, PeerCountUpdate::Remove);

            // Set peerConnection to CONNECTED state.
            self.connections.get_mut(connection_id).unwrap().set_network_connection(connection);
        } else {
            // Add connection (without having obtained peer address).
            connection_id = self.add(ConnectionInfo::inbound(connection));
            self.inbound_count += 1;
        }

        // Register close listener early to clean up correctly in case _checkConnection() closes the connection.
        let info = self.connections.get(connection_id).expect("Missing connection");
        let peer_channel = PeerChannel::new(info.network_connection().unwrap());
        peer_channel.notifier.write().register(weak_listener(self.listener.clone(), move |arc, event| {
            arc.write().on_peer_channel_event(connection_id, event);
        }));

        if !self.check_connection(connection_id) {
            return;
        }

        // Connection accepted.

        let net_address = info.network_connection().map(|p| p.net_address()).clone();

        if let Some(ref net_address) = net_address {
            self.add_net_address(connection_id, &net_address);
        }

        // The extra lookup is needed to satisfy the borrow checker.
        let info = self.connections.get_mut(connection_id).expect("Missing connection");

        let conn_type = if info.network_connection().unwrap().inbound() { "inbound" } else { "outbound" };
        debug!("Connection established ({}) #{} (net_address={:?}, peer_address={:?})", conn_type, connection_id, net_address, info.peer_address());

        // Let listeners know about this connection.
        // TODO self.notifier.notify(ConnectionPoolEvent::Connection(...);

        // Set the peer_channel
        info.set_peer_channel(peer_channel);

        // TODO create agent and initate handshake
    }

    /// Checks the validity of a handshake.
    fn check_handshake(&mut self, connection_id: ConnectionId, peer: &Peer<'network>) -> bool {
        let info = self.connections.get(connection_id).unwrap();

        // Close connection if peer's address is banned.
        if self.addresses.is_banned(peer.peer_address()) {
            ConnectionPool::close(info.network_connection(), CloseType::PeerIsBanned);
            return false;
        }

        // Duplicate/simultaneous connection check (post version):
        let stored_connection_id = self.connections_by_peer_address.get(&peer.peer_address());
        if let Some(stored_connection_id) = stored_connection_id {
            if *stored_connection_id != connection_id {
                // If we already have an established connection to this peer, close this connection.
                let stored_connection = self.connections.get(*stored_connection_id).expect("Missing connection");
                if stored_connection.state() == ConnectionState::Established {
                    ConnectionPool::close(info.network_connection(), CloseType::DuplicateConnection);
                    return false;
                }
            }
        }

        // Close connection if we have too many dumb connections.
        if peer.peer_address().protocol() == Protocol::Dumb && self.peer_count_dumb >= network::PEER_COUNT_DUMB_MAX {
            ConnectionPool::close(info.network_connection(), CloseType::ConnectionLimitDumb);
            return false;
        }

        // Set peerConnection to NEGOTIATING state.
        self.connections.get_mut(connection_id).unwrap().negotiating();

        return false;
    }

    /// Callback during handshake.
    fn on_handshake(&mut self, connection_id: ConnectionId, peer: Peer<'network>) {
        let info = self.connections.get(connection_id).expect("Missing connection");
        let network_connection = info.network_connection().unwrap();

        if network_connection.inbound() {
            // Re-check allowInboundExchange as it might have changed.
            if self.peer_count() >= network::PEER_COUNT_MAX && !self.allow_inbound_exchange {
                ConnectionPool::close(info.network_connection(), CloseType::MaxPeerCountReached);
                return;
            }

            // Duplicate/simultaneous connection check (post handshake):
            let stored_connection_id = self.connections_by_peer_address.get(&peer.peer_address());
            if let Some(stored_connection_id) = stored_connection_id {
                if *stored_connection_id != connection_id {
                    let stored_connection = self.connections.get(*stored_connection_id).expect("Missing connection");
                    match stored_connection.state() {
                        ConnectionState::Connecting => {
                            // Abort the stored connection attempt and accept this connection.
                            let protocol = peer.peer_address().protocol();
                            assert!(protocol == Protocol::Wss || protocol == Protocol::Ws, "Duplicate connection to non-WS node");
                            debug!("Aborting connection attempt to {:?}, simultaneous connection succeeded", peer.peer_address());

                            // TODO abort connecting
                            assert!(self.get_connection_by_peer_address(&peer.peer_address()).is_none(), "ConnectionInfo not removed");
                        },
                        ConnectionState::Established => {
                            // If we have another established connection to this peer, close this connection.
                            ConnectionPool::close(info.network_connection(), CloseType::DuplicateConnection);
                            return;
                        },
                        ConnectionState::Negotiating => {
                            // The peer with the lower peerId accepts this connection and closes his stored connection.
                            // TODO get own PeerId and compare
                            // if <self>.peer_address().peer_id() < peer.peer_address().peer_id() {
                            if true {
                                ConnectionPool::close(stored_connection.network_connection(), CloseType::SimultaneousConnection);
                                assert!(self.get_connection_by_peer_address(&peer.peer_address()).is_none(), "ConnectionInfo not removed");
                            } else {
                                // The peer with the higher peerId closes this connection and keeps his stored connection.
                                ConnectionPool::close(info.network_connection(), CloseType::SimultaneousConnection);
                            }
                        },
                        _ => {
                            // Accept this connection and close the stored connection.
                            ConnectionPool::close(stored_connection.network_connection(), CloseType::SimultaneousConnection);
                            assert!(self.get_connection_by_peer_address(&peer.peer_address()).is_none(), "ConnectionInfo not removed");
                        },
                    }
                }
            }

            assert!(self.get_connection_by_peer_address(&peer.peer_address()).is_none(), "ConnectionInfo already exists");
            self.connections.get_mut(connection_id).unwrap().set_peer_address(peer.peer_address());
            self.add_peer_address(connection_id, peer.peer_address());

            self.inbound_count = self.inbound_count.checked_sub(1).expect("inbound_count < 0");
        }

        // Handshake accepted.

        // Check if we need to recycle a connection.
        if self.peer_count() >= network::PEER_COUNT_MAX {
             self.notifier.notify(ConnectionPoolEvent::RecyclingRequest);
        }

        // Set ConnectionInfo to Established state.
        self.connections.get_mut(connection_id).unwrap().set_peer(peer.clone()); // TODO do we need a clone here?

        if let Some(net_address) = peer.net_address() {
            // The HashSet takes care of only inserting it once.
            self.add_net_address(connection_id, &net_address);
        }

        self.update_connected_peer_count(connection_id, PeerCountUpdate::Add);

        // TODO Setup signal forwarding.

        // Mark address as established.
        // TODO Maybe replace Session by Peer?
        let info = self.connections.get(connection_id).expect("Missing connection");
        let network_connection = info.network_connection().unwrap();
        // TODO self.addresses.established(&network_connection.session(), peer.peer_address());

        // Let listeners know about this peer.
        self.notifier.notify(ConnectionPoolEvent::PeerJoined(peer.clone()));

        // Let listeners know that the peers changed.
        self.notifier.notify(ConnectionPoolEvent::PeersChanged);

        debug!("[PEER-JOINED] {:?} {:?} (version={:?}, services={:?}, headHash={:?})", peer.peer_address(), peer.net_address(), peer.version, peer.peer_address().services, peer.head_hash);
    }

    /// Callback upon closing of connection.
    fn on_close(&mut self, connection_id: ConnectionId, ty: CloseType) {
        // Only propagate the close type (i.e. track fails/bans) if the peerAddress is set.
        // This is true for
        // - all outbound connections
        // - inbound connections post handshake (peerAddress is verified)
        let info = self.connections.get(connection_id).unwrap();
        if let Some(peer_address) = info.peer_address() {
            // TODO self.addresses.close(&info.session().unwrap(), peer_address, ty);
        }

        let mut info = self.remove(connection_id);

        // Check if the handshake with this peer has completed.
        if info.state() == ConnectionState::Established {
            let net_address = info.network_connection().map(|p| p.net_address());
            // If closing is due to a ban, also ban the IP
            if ty.is_banning_type() {
                if let Some(ref net_address) = net_address {
                    self.ban_ip(net_address);
                }
            }

            self.update_connected_peer_count(connection_id, PeerCountUpdate::Remove);

            // Tell listeners that this peer has gone away.
            self.notifier.notify(ConnectionPoolEvent::PeerLeft(info.peer().expect("Peer not set").clone()));

            // Let listeners know that the peers changed.
            self.notifier.notify(ConnectionPoolEvent::PeersChanged);

            debug!("[PEER-LEFT] {:?} {:?} (version={:?}, closeType={:?})", info.peer_address(), net_address, info.peer().map(|p| p.version), ty);
        } else {
            match info.network_connection().map(|n| n.inbound()) {
                Some(true) => {
                    self.inbound_count.checked_sub(1).expect("inbound_count < 0");
                    debug!("Inbound connection #{:?} closed pre-handshake: {:?}", connection_id, ty);
                },
                Some(false) => {
                    debug!("Connection #{:?} to {:?} closed pre-handshake: {:?}", connection_id, info.peer_address(), ty);
                    self.notifier.notify(ConnectionPoolEvent::ConnectError(info.peer_address().expect("PeerAddress not set").clone(), ty));
                },
                _ => unreachable!("Invalid state, closing connection with network connection not set"),
            }
        }

        // Let listeners know about this closing.
        self.notifier.notify(ConnectionPoolEvent::Close(connection_id, ty));

        // Set the peer connection to closed state.
        info.close();
    }

    /// Total peer count.
    pub fn peer_count(&self) -> usize {
        self.peer_count_ws + self.peer_count_wss + self.peer_count_rtc + self.peer_count_dumb
    }

    /// Bans an IP address.
    fn ban_ip(&mut self, net_address: &NetAddress) {
        if net_address.is_reliable() {
            warn!("Banning ip {:?}", net_address);
            let banned_address = if net_address.get_type() == NetAddressType::IPv4 {
                net_address.clone()
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
        let mut now = SystemTime::now();
        self.banned_ips.retain(|net_address, unban_time| {
            unban_time > &mut now
        });
    }

    /// Callback on connect error.
    fn on_connect_error(&mut self, peer_address: Arc<PeerAddress>) {
        debug!("Connection to {:?} failed", peer_address);

        let connection_id = *self.connections_by_peer_address.get(&peer_address).expect("PeerAddress not stored");
        let info = self.connections.get(connection_id).expect("Missing connection");
        assert_eq!(info.state(), ConnectionState::Connecting, "ConnectionInfo state not Connecting, but {:?} ({:?})", info.state(), peer_address);
        self.remove(connection_id);

        self.connecting_count = self.connecting_count.checked_sub(1).expect("connecting_count < 0");

        // TODO: PeerAddressBook currently doesn't support optional first argument.
//        self.addresses.close(None, peer_address, CloseType::ConnectionFailed);

        self.notifier.notify(ConnectionPoolEvent::ConnectError(peer_address.clone(), CloseType::ConnectionFailed));
    }

    /// Updates the number of connected peers.
    fn update_connected_peer_count(&mut self, connection_id: ConnectionId, update: PeerCountUpdate) {
        // We assume the connection to be present and having a valid peer address/network connection.
        let info = self.connections.get(connection_id).unwrap();
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
            if peer_address.services.is_full_node() && (peer_address.protocol() == Protocol::Wss || peer_address.protocol() == Protocol::Ws) {
                update_checked!(self.peer_count_full_ws_outbound, update);
            }
        }
    }

    /// Convert a net address into a subnet according to the configured bitmask.
    fn get_subnet_address(net_address: &NetAddress) -> NetAddress {
        let bit_mask = if net_address.get_type() == NetAddressType::IPv4 { network::IPV4_SUBNET_MASK } else { network::IPV6_SUBNET_MASK };
        net_address.subnet(bit_mask)
    }

    /// Check the validity of a outbound connection request (e.g. no duplicate connections).
    fn check_outbound_connection_request(&self, peer_address: Arc<PeerAddress>) -> bool {
        match peer_address.protocol() {
            Protocol::Wss => {},
            Protocol::Ws => {},
            _ => {
                error!("Cannot connect to {} - unsupported protocol", peer_address);
                return false;
            },
        }

        if self.addresses.is_banned(peer_address.clone()) {
            error!("Connecting to banned address {:?}", peer_address);
            return false;
        }

        let info = self.get_connection_by_peer_address(&peer_address);
        if let Some(info) = info {
            error!("Duplicate connection to {}", peer_address);
            return false;
        }

        // Forbid connection if we have too many connections to the peer's IP address.
        if peer_address.net_address.is_reliable() {
            if self.get_num_connections_by_net_address(&peer_address.net_address) >= network::PEER_COUNT_PER_IP_MAX {
                error!("Connection limit per IP ({}) reached", network::PEER_COUNT_PER_IP_MAX);
                return false;
            }

            if self.get_num_outbound_connections_by_subnet(&peer_address.net_address) >= network::OUTBOUND_PEER_COUNT_PER_SUBNET_MAX {
                error!("Connection limit per IP ({}) reached", network::OUTBOUND_PEER_COUNT_PER_SUBNET_MAX);
                return false;
            }
        }

        return true;
    }

    /// Add a new connection to the connection pool.
    fn add(&mut self, info: ConnectionInfo<'network>) -> ConnectionId {
        let peer_address = info.peer_address();
        let connection_id = self.connections.insert(info);

        // Add to peer address map if available.
        if let Some(peer_address) = peer_address {
            self.connections_by_peer_address.insert(peer_address, connection_id);
        }
        connection_id
    }

    /// Add a new connection to the connection pool.
    fn add_peer_address(&mut self, connection_id: ConnectionId, peer_address: Arc<PeerAddress>) {
        // Add to peer address map.
        self.connections_by_peer_address.insert(peer_address, connection_id);
    }

    /// Remove a connection from the connection pool.
    fn remove(&mut self, connection_id: ConnectionId) -> ConnectionInfo<'network> {
        // TODO: Can we make sure that we never remove a connection twice?
        let info = self.connections.remove(connection_id).unwrap();

        if let Some(peer_address) = info.peer_address() {
            self.connections_by_peer_address.remove(&peer_address);
        }

        if let Some(network_connection) = info.network_connection() {
            self.remove_net_address(connection_id, &network_connection.net_address());
        }

        info
    }

    /// Adds the net address to a connection.
    fn add_net_address(&mut self, connection_id: ConnectionId, net_address: &NetAddress) {
        // Only add reliable netAddresses.
        if !net_address.is_reliable() {
            return;
        }

        self.connections_by_net_address.entry(net_address.clone())
            .or_insert_with(HashSet::new)
            .insert(connection_id);

        let subnet_address = ConnectionPool::get_subnet_address(net_address);
        self.connections_by_subnet.entry(subnet_address)
            .or_insert_with(HashSet::new)
            .insert(connection_id);
    }

    /// Removes the connection from net address specific maps.
    fn remove_net_address(&mut self, connection_id: ConnectionId, net_address: &NetAddress) {
        // Only add reliable netAddresses.
        if !net_address.is_reliable() {
            return;
        }

        if let Entry::Occupied(mut occupied) = self.connections_by_net_address.entry(net_address.clone()) {
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
}

enum PeerCountUpdate {
    Add,
    Remove
}

#[derive(Clone)]
enum ConnectionPoolEvent<'network> {
    PeerJoined(Peer<'network>),
    PeerLeft(Peer<'network>),
    PeersChanged,
    ConnectError(Arc<PeerAddress>, CloseType),
    Close(ConnectionId, CloseType), // TODO is that really useful? ConnectionId won't exist anymore
//    Connection(NetworkConnection), // TODO not really practical
    RecyclingRequest,
}

/// This is a special vector implementation that has a O(1) remove function.
/// It never shrinks in size, but reuses available spaces as much as possible.
struct SparseVec<T> {
    inner: Vec<Option<T>>,
    free_indices: LinkedList<usize>,
}

impl<T> SparseVec<T> {
    pub fn new() -> Self {
        SparseVec {
            inner: Vec::new(),
            free_indices: LinkedList::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        SparseVec {
            inner: Vec::with_capacity(capacity),
            free_indices: LinkedList::new(),
        }
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.inner.get(index)?.as_ref()
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.inner.get_mut(index)?.as_mut()
    }

    pub fn remove(&mut self, index: usize) -> Option<T> {
        let value = self.inner.get_mut(index)?.take();
        if value.is_some() {
            self.free_indices.push_back(index);
        }
        value
    }

    pub fn insert(&mut self, value: T) -> usize {
        if let Some(index) = self.free_indices.pop_front() {
            self.inner.get_mut(index).unwrap().get_or_insert(value);
            index
        } else {
            let index = self.inner.len();
            self.inner.push(Some(value));
            index
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_vec_can_store_objects() {
        let mut v = SparseVec::new();

        // Insert.
        let i1 = v.insert(5);
        assert_eq!(i1, 0);
        let i2 = v.insert(5);
        assert_eq!(i2, 1);

        // Read/Write access.
        assert_eq!(v.get(i1), Some(&5));
        *v.get_mut(i2).unwrap() = 8;
        assert_eq!(v.get(i2), Some(&8));
        assert_eq!(v.get(2), None);
        assert_eq!(v.free_indices.len(), 0);

        // Remove.
        assert_eq!(v.remove(i1), Some(5));
        assert_eq!(v.get(i1), None);
        let i3 = v.insert(1);
        assert_eq!(i3, 0);

        assert_eq!(v.remove(i2), Some(8));
        assert_eq!(v.remove(i2), None);
        assert_eq!(v.free_indices.len(), 1);

        let i4 = v.insert(2);
        assert_eq!(i4, 1);
        assert_eq!(v.free_indices.len(), 0);

        let i5 = v.insert(4);
        assert_eq!(i5, 2);
    }
}
