use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::StreamExt;
use libp2p::{
    core::Endpoint,
    identity::Keypair,
    swarm::{
        behaviour::{ConnectionClosed, ConnectionEstablished},
        CloseConnection, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, ToSwarm,
    },
    Multiaddr, PeerId,
};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;
use nimiq_time::{interval, Interval};
use parking_lot::RwLock;

use super::{
    handler::{Handler, HandlerOutEvent},
    peer_contacts::{PeerContact, PeerContactBook},
};

#[derive(Clone, Debug)]
pub struct Config {
    /// Genesis hash for the network we want to be connected to.
    pub genesis_hash: Blake2bHash,

    /// Interval in which we want to be updated.
    pub update_interval: Duration,

    /// Minimum update interval, that we will accept. If peer contact updates are received faster than this, they will
    /// be rejected.
    pub min_recv_update_interval: Duration,

    /// How many updated peer contacts we want to receive per update.
    pub update_limit: u16,

    /// Services for which we filter (the services that we need others to provide)
    pub required_services: Services,

    /// Minimum interval that we will update other peers with.
    pub min_send_update_interval: Duration,

    /// Interval in which the peer address book is cleaned up.
    pub house_keeping_interval: Duration,

    /// Whether to keep the connection alive, even if no other behaviour uses it.
    pub keep_alive: bool,

    /// Only secure websocket connections
    pub only_secure_ws_connections: bool,
}

impl Config {
    pub fn new(
        genesis_hash: Blake2bHash,
        required_services: Services,
        only_secure_ws_connections: bool,
    ) -> Self {
        Self {
            genesis_hash,
            update_interval: Duration::from_secs(60),
            min_send_update_interval: Duration::from_secs(30),
            min_recv_update_interval: Duration::from_secs(25),
            update_limit: 64,
            required_services,
            house_keeping_interval: Duration::from_secs(25),
            keep_alive: true,
            only_secure_ws_connections,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Established {
        peer_id: PeerId,
        peer_address: Multiaddr,
        peer_contact: PeerContact,
    },
    Update,
}

type DiscoveryToSwarm = ToSwarm<Event, ()>;

/// Network behaviour for peer exchange.
///
/// When a connection to a peer is established, a handshake is done to exchange protocols and services filters, and
/// subscription settings. The peers then send updates to each other in a configurable interval.
///
/// # TODO
///
///  - Exchange clock time with other peers.
///
pub struct Behaviour {
    /// Configuration for the discovery behaviour
    config: Config,

    /// Identity key pair
    keypair: Keypair,

    /// `PeerId`s of all connected peers.
    connected_peers: HashSet<PeerId>,

    /// Contains all known peer contacts.
    peer_contact_book: Arc<RwLock<PeerContactBook>>,

    /// Queue with events to emit.
    pub events: VecDeque<DiscoveryToSwarm>,

    /// Timer to do house-keeping in the peer address book.
    house_keeping_timer: Interval,
}

impl Behaviour {
    pub fn new(
        config: Config,
        keypair: Keypair,
        peer_contact_book: Arc<RwLock<PeerContactBook>>,
    ) -> Self {
        let house_keeping_timer = interval(config.house_keeping_interval);
        peer_contact_book.write().update_own_contact(&keypair);

        // Report our own known addresses as candidates to the swarm
        let mut events = VecDeque::new();
        for address in peer_contact_book.read().get_own_contact().addresses() {
            events.push_back(ToSwarm::NewExternalAddrCandidate(address.clone()));
        }

        Self {
            config,
            keypair,
            connected_peers: HashSet::new(),
            peer_contact_book,
            events,
            house_keeping_timer,
        }
    }

    /// Adds addresses into our own contact within the peer contact book
    pub fn add_own_addresses(&self, addresses: Vec<Multiaddr>) {
        self.peer_contact_book
            .write()
            .add_own_addresses(addresses, &self.keypair)
    }

    /// Returns whether an address in `Multiaddr` format is a dialable websocket address
    pub fn is_address_dialable(&self, address: &Multiaddr) -> bool {
        self.peer_contact_book.read().is_address_dialable(address)
    }

    /// Returns a reference to the peer contact book
    fn peer_contact_book(&self) -> Arc<RwLock<PeerContactBook>> {
        Arc::clone(&self.peer_contact_book)
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Handler;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer: PeerId,
        _local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<Handler, ConnectionDenied> {
        Ok(Handler::new(
            peer,
            self.config.clone(),
            self.keypair.clone(),
            self.peer_contact_book(),
            remote_addr.clone(),
        ))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        _role_override: Endpoint,
    ) -> Result<Handler, ConnectionDenied> {
        Ok(Handler::new(
            peer,
            self.config.clone(),
            self.keypair.clone(),
            self.peer_contact_book(),
            addr.clone(),
        ))
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
            .peer_contact_book
            .read()
            .get_addresses(&peer_id)
            .unwrap_or_default())
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<DiscoveryToSwarm> {
        // Emit events
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        // Poll house-keeping timer
        match self.house_keeping_timer.poll_next_unpin(cx) {
            Poll::Ready(Some(_)) => {
                trace!("Doing house-keeping in peer address book");
                let mut peer_address_book = self.peer_contact_book.write();
                peer_address_book.update_own_contact(&self.keypair);
                peer_address_book.house_keeping();
            }
            Poll::Ready(None) => unreachable!(),
            Poll::Pending => {}
        }

        Poll::Pending
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionClosed(ConnectionClosed {
                peer_id,
                remaining_established,
                ..
            }) => {
                if remaining_established == 0 {
                    // There are no more remaining connections to this peer
                    self.connected_peers.remove(&peer_id);
                }
            }
            FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                failed_addresses,
                other_established,
            }) => {
                if other_established == 0 {
                    trace!(%peer_id, ?connection_id, ?endpoint, "Behaviour::inject_connection_established:");

                    // This is the first connection to this peer
                    self.connected_peers.insert(peer_id);

                    if endpoint.is_listener() {
                        // Peer failed to connect with some of our own addresses, remove them from our own addresses
                        if !failed_addresses.is_empty() {
                            debug!(
                                ?failed_addresses,
                                "Removing failed address from own addresses"
                            );
                            self.peer_contact_book.write().remove_own_addresses(
                                failed_addresses.iter().cloned(),
                                &self.keypair,
                            )
                        }
                    }
                } else {
                    trace!(%peer_id, "Behaviour::inject_connection_established: Already have a connection established to peer");
                }
            }
            _ => {}
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        _connection: ConnectionId,
        event: HandlerOutEvent,
    ) {
        trace!(%peer_id, ?event, "on_connection_handler_event");

        match event {
            HandlerOutEvent::PeerExchangeEstablished {
                peer_address,
                peer_contact: signed_peer_contact,
            } => {
                if let Some(peer_contact) = self.peer_contact_book.read().get(&peer_id) {
                    self.events
                        .push_back(ToSwarm::GenerateEvent(Event::Established {
                            peer_id: signed_peer_contact.public_key().clone().to_peer_id(),
                            peer_address,
                            peer_contact: peer_contact.contact().clone(),
                        }));
                }
            }
            HandlerOutEvent::ObservedAddress { observed_address } => {
                self.events
                    .push_back(ToSwarm::NewExternalAddrCandidate(observed_address));
            }
            HandlerOutEvent::Update => self.events.push_back(ToSwarm::GenerateEvent(Event::Update)),
            HandlerOutEvent::Error(_) => self.events.push_back(ToSwarm::CloseConnection {
                peer_id,
                connection: CloseConnection::All,
            }),
        }
    }
}
