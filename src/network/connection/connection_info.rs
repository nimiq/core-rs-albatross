use std::sync::Arc;

use crate::network::address::peer_address::PeerAddress;
use crate::network::connection::NetworkConnection;
use crate::network::Peer;
use crate::network::peer_channel::Session;

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
    state: ConnectionState,
}

impl ConnectionInfo {
    pub fn new() -> Self {
        ConnectionInfo {
            peer_address: None,
            network_connection: None,
            peer: None,
            state: ConnectionState::New,
        }
    }

    pub fn inbound(connection: NetworkConnection) -> Self {
        let mut info = ConnectionInfo::new();
        info.set_network_connection(connection);
        info
    }

    pub fn outbound(peer_address: PeerAddress) -> Self {
        let mut info = ConnectionInfo::new();
        info.peer_address = Some(Arc::new(peer_address));
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
    pub fn session(&self) -> Option<Arc<Session>> { self.network_connection.as_ref().map(|n| n.session()) }

    pub fn set_peer_address(&mut self, peer_address: Arc<PeerAddress>) { self.peer_address = Some(peer_address) }
    pub fn set_network_connection(&mut self, network_connection: NetworkConnection) {
        self.network_connection = Some(network_connection);
        self.state = ConnectionState::Connected;
    }
    pub fn set_peer(&mut self, peer: Peer) {
        self.peer = Some(peer);
        self.state = ConnectionState::Established;
    }

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
