use std::{sync::Arc, time::{Duration, Instant}};
use std::collections::HashMap;

use parking_lot::RwLock;

use network_primitives::address::peer_address::PeerAddress;
use network_primitives::message::Message;
use network_primitives::message::MessageType;

use crate::connection::network_agent::NetworkAgent;
use crate::connection::NetworkConnection;
use crate::Peer;
use crate::peer_channel::PeerChannel;
use crate::websocket::websocket_connector::ConnectionHandle;

#[derive(Copy, Clone, Debug, Ord, PartialOrd, PartialEq, Eq)]
pub enum ConnectionState {
    New,
    Connecting,
    Connected,
    Negotiating,
    Established,
    Closed
}

pub struct ConnectionInfo {
    peer_address: Option<Arc<PeerAddress>>,
    network_connection: Option<NetworkConnection>,
    peer: Option<Peer>,
    peer_channel: Option<PeerChannel>,
    state: ConnectionState,
    network_agent: Option<Arc<RwLock<NetworkAgent>>>,
    connection_handle: Option<Arc<ConnectionHandle>>,
    established_since: Option<Instant>,
    statistics: ConnectionStatistics,
}

impl ConnectionInfo {
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
    pub fn peer_channel(&self) -> Option<&PeerChannel> { self.peer_channel.as_ref() }
    pub fn network_agent(&self) -> Option<&Arc<RwLock<NetworkAgent>>> { self.network_agent.as_ref() }
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
    pub fn set_peer_channel(&mut self, peer_channel: PeerChannel) { self.peer_channel = Some(peer_channel); }
    pub fn set_network_agent(&mut self, network_agent: Arc<RwLock<NetworkAgent>>) { self.network_agent = Some(network_agent); }
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

impl PartialEq for ConnectionInfo {
    fn eq(&self, other: &ConnectionInfo) -> bool {
        self.peer_address == other.peer_address
    }
}

impl Eq for ConnectionInfo {}

pub struct ConnectionStatistics {
    latencies: Vec<f32>,
    messages: HashMap<MessageType, u32>,
}

impl ConnectionStatistics {
    pub fn new() -> Self {
        Self {
            latencies: Vec::new(),
            messages: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        // FIXME: This should be optimized
        self.latencies = Vec::new();
        self.messages = HashMap::new();
    }

    pub fn add_latency(&mut self, latency: f32) {
        self.latencies.push(latency);
    }

    pub fn add_message(&mut self, message: Message) {
        unimplemented!()
    }

    pub fn message_count(&self, message_type: MessageType) {
        unimplemented!()
    }

    pub fn latency_median(&self) -> f32 {
        // FIXME: Cloning is slow but needed to sort this vector without requiring &mut self (which we dont have in PeerScorer use)
        let mut latencies = self.latencies.clone();
        let length = self.latencies.len();

        if length == 0 { return 0.0; }

        latencies.sort_unstable_by(|a, b| a.partial_cmp(&b).unwrap());

        let median = match length % 2 {
            0 => ((latencies[(length / 2) - 1] + latencies[length / 2]) / 2.0).round(),
            1 => latencies[(length - 1) / 2],
            _ => unreachable!()
        };

        return median;
    }
}
