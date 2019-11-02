use std::{fmt, sync::Arc, time::{Duration, Instant}};

use parking_lot::RwLock;

use blockchain_base::AbstractBlockchain;
use network_primitives::address::peer_address::PeerAddress;

use crate::connection::network_agent::NetworkAgent;
use crate::connection::NetworkConnection;
use crate::Peer;
use crate::peer_channel::PeerChannel;
use crate::websocket::websocket_connector::ConnectionHandle;

#[derive(Copy, Clone, Debug, Ord, PartialOrd, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    New = 1,
    Connecting = 2,
    Connected = 3,
    Negotiating = 4,
    Established = 5,
    Closed = 6
}

pub struct ConnectionInfo<B: AbstractBlockchain + 'static> {
    peer_address: Option<Arc<PeerAddress>>,
    network_connection: Option<NetworkConnection>,
    peer: Option<Peer>,
    peer_channel: Option<Arc<PeerChannel>>,
    state: ConnectionState,
    network_agent: Option<Arc<RwLock<NetworkAgent<B>>>>,
    connection_handle: Option<Arc<ConnectionHandle>>,
    established_since: Option<Instant>,
    statistics: ConnectionStatistics,
}

impl<B: AbstractBlockchain + 'static> ConnectionInfo<B> {
    pub fn new() -> Self {
        ConnectionInfo {
            peer_address: None,
            network_connection: None,
            peer: None,
            peer_channel: None,
            state: ConnectionState::New,
            network_agent: None,
            connection_handle: None,
            established_since: None,
            statistics: ConnectionStatistics::new(),
        }
    }

    pub fn inbound(connection: NetworkConnection) -> Self {
        let mut info = ConnectionInfo::new();
        info.set_network_connection(connection);
        info
    }

    pub fn outbound(peer_address: Arc<PeerAddress>) -> Self {
        let mut info = ConnectionInfo::new();
        info.peer_address = Some(peer_address);
        info.state = ConnectionState::Connecting;
        info
    }

    pub fn state(&self) -> ConnectionState { self.state }
    pub fn peer_address(&self) -> Option<Arc<PeerAddress>> {
        if let Some(ref peer_address) = self.peer_address {
            Some(peer_address.clone())
        } else {
            None
        }
    }
    pub fn network_connection(&self) -> Option<&NetworkConnection> { self.network_connection.as_ref() }
    pub fn peer(&self) -> Option<&Peer> { self.peer.as_ref() }
    pub fn peer_channel(&self) -> Option<Arc<PeerChannel>> { self.peer_channel.clone() }
    pub fn network_agent(&self) -> Option<&Arc<RwLock<NetworkAgent<B>>>> { self.network_agent.as_ref() }
    pub fn connection_handle(&self) -> Option<&Arc<ConnectionHandle>> { self.connection_handle.as_ref() }
    pub fn age_established(&self) -> Duration { self.established_since.expect("No peer has been set yet").elapsed() }
    pub fn statistics(&self) -> &ConnectionStatistics { &self.statistics }

    pub fn set_peer_address(&mut self, peer_address: Arc<PeerAddress>) { self.peer_address = Some(peer_address) }
    pub fn set_network_connection(&mut self, network_connection: NetworkConnection) {
        self.network_connection = Some(network_connection);
        self.state = ConnectionState::Connected;
    }
    pub fn set_peer(&mut self, peer: Peer) {
        self.peer = Some(peer);
        self.state = ConnectionState::Established;
        self.established_since = Some(Instant::now());
    }
    pub fn set_peer_channel(&mut self, peer_channel: Arc<PeerChannel>) { self.peer_channel = Some(peer_channel); }
    pub fn set_network_agent(&mut self, network_agent: Arc<RwLock<NetworkAgent<B>>>) { self.network_agent = Some(network_agent); }
    pub fn set_connection_handle(&mut self, handle: Arc<ConnectionHandle>) { self.connection_handle = Some(handle); }
    pub fn drop_connection_handle(&mut self) { self.connection_handle = None; }

    pub fn negotiating(&mut self) {
        assert_eq!(self.state, ConnectionState::Connected);
        self.state = ConnectionState::Negotiating;
    }

    pub fn close(&mut self) {
        self.state = ConnectionState::Closed;
        self.network_connection = None;
        self.peer = None;
    }
}

impl<B: AbstractBlockchain + 'static> Default for ConnectionInfo<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: AbstractBlockchain + 'static> PartialEq for ConnectionInfo<B> {
    fn eq(&self, other: &ConnectionInfo<B>) -> bool {
        self.peer_address == other.peer_address
    }
}

impl<B: AbstractBlockchain + 'static> Eq for ConnectionInfo<B> {}

impl<B: AbstractBlockchain + 'static> fmt::Display for ConnectionInfo<B> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{{ peer_address: {}, state: {:?} }}", match self.peer_address {
            Some(ref peer_address) => peer_address.to_string(),
            None => "None".to_string(),
        }, self.state)
    }
}

pub struct ConnectionStatistics {
    latencies: Vec<f64>,
}

impl ConnectionStatistics {
    pub fn new() -> Self {
        Self {
            latencies: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.latencies.clear();
    }

    pub fn add_latency(&mut self, latency: f64) {
        self.latencies.push(latency);
    }

    pub fn latency_median(&self) -> f64 {
        // FIXME: Cloning is slow but needed to sort this vector without requiring &mut self (which we don't have in PeerScorer use)
        let mut latencies = self.latencies.clone();
        let length = self.latencies.len();

        if length == 0 { return 0.0; }

        latencies.sort_unstable_by(|a, b| a.partial_cmp(&b).unwrap());

        match length % 2 {
            0 => ((latencies[(length / 2) - 1] + latencies[length / 2]) / 2.0).round(),
            1 => latencies[(length - 1) / 2],
            _ => unreachable!()
        }
    }
}

impl Default for ConnectionStatistics {
    fn default() -> Self {
        Self::new()
    }
}
