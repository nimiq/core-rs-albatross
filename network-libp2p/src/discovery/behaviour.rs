use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use futures::{
    task::{Context, Poll},
    StreamExt,
};
use libp2p::{
    core::connection::{ConnectedPoint, ConnectionId},
    identity::Keypair,
    swarm::{AddressScore, KeepAlive, NetworkBehaviour, NetworkBehaviourAction, PollParameters},
    Multiaddr, PeerId,
};
use parking_lot::RwLock;
use wasm_timer::Interval;

use nimiq_hash::Blake2bHash;
use nimiq_utils::time::OffsetTime;

use super::{
    handler::{DiscoveryHandler, HandlerInEvent, HandlerOutEvent},
    peer_contacts::{PeerContactBook, Protocols, Services},
};

#[derive(Clone, Debug)]
pub struct DiscoveryConfig {
    /// Genesis hash for the network we want to be connected to.
    pub genesis_hash: Blake2bHash,

    /// Interval in which we want to be updated.
    pub update_interval: Duration,

    /// Minimum update interval, that we will accept. If peer contact updates are received faster than this, they will
    /// be rejected.
    pub min_recv_update_interval: Duration,

    /// How many updated peer contacts we want to receive per update.
    pub update_limit: u16,

    /// Protocols for which we filter.
    pub protocols_filter: Protocols,

    /// Services for which we filter.
    pub services_filter: Services,

    /// Minimium interval that we will update other peers with.
    pub min_send_update_interval: Duration,

    /// Interval in which the peer address book is cleaned up.
    pub house_keeping_interval: Duration,

    /// Whether to keep the connection alive, even if no other behaviour uses it.
    pub keep_alive: KeepAlive,
}

impl DiscoveryConfig {
    pub fn new(genesis_hash: Blake2bHash) -> Self {
        Self {
            genesis_hash,
            update_interval: Duration::from_secs(60),
            min_send_update_interval: Duration::from_secs(30),
            min_recv_update_interval: Duration::from_secs(30),
            update_limit: 64,
            protocols_filter: Protocols::all(),
            services_filter: Services::all(),
            house_keeping_interval: Duration::from_secs(60),
            keep_alive: KeepAlive::Yes,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DiscoveryEvent {
    Established { peer_id: PeerId },
    Update,
}

/// Network behaviour for peer exchange.
///
/// When a connection to a peer is established, a handshake is done to exchange protocols and services filters, and
/// subscription settings. The peers then send updates to each other in a configurable interval.
///
/// # TODO
///
///  - Exchange clock time with other peers.
///
pub struct DiscoveryBehaviour {
    /// Configuration for the discovery behaviour
    config: DiscoveryConfig,

    /// Identity key pair
    keypair: Keypair,

    /// `PeerId`s of all connected peers.
    connected_peers: HashSet<PeerId>,

    /// Contains all known peer contacts.
    peer_contact_book: Arc<RwLock<PeerContactBook>>,

    #[allow(dead_code)]
    clock: Arc<OffsetTime>,

    /// Queue with events to emit.
    pub events: VecDeque<NetworkBehaviourAction<HandlerInEvent, DiscoveryEvent>>,

    /// Timer to do house-keeping in the peer address book.
    house_keeping_timer: Interval,
}

impl DiscoveryBehaviour {
    pub fn new(
        config: DiscoveryConfig,
        keypair: Keypair,
        peer_contact_book: Arc<RwLock<PeerContactBook>>,
        clock: Arc<OffsetTime>,
    ) -> Self {
        let house_keeping_timer = Interval::new(config.house_keeping_interval);
        peer_contact_book.write().self_update(&keypair);

        Self {
            config,
            keypair,
            connected_peers: HashSet::new(),
            peer_contact_book,
            clock,
            events: VecDeque::new(),
            house_keeping_timer,
        }
    }

    pub fn peer_contact_book(&self) -> Arc<RwLock<PeerContactBook>> {
        Arc::clone(&self.peer_contact_book)
    }
}

impl NetworkBehaviour for DiscoveryBehaviour {
    type ProtocolsHandler = DiscoveryHandler;
    type OutEvent = DiscoveryEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        DiscoveryHandler::new(
            self.config.clone(),
            self.keypair.clone(),
            self.peer_contact_book(),
        )
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<Multiaddr> {
        let addresses = self
            .peer_contact_book
            .read()
            .get(peer_id)
            .map(|addresses_opt| addresses_opt.addresses().cloned().collect())
            .unwrap_or_default();

        log::debug!("addresses_of_peer({}) = {:#?}", peer_id, addresses);

        addresses
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        self.connected_peers.insert(*peer_id);
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        self.connected_peers.remove(peer_id);
    }

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        connected_point: &ConnectedPoint,
    ) {
        log::trace!("DiscoveryBehaviour::inject_connection_established:");
        log::trace!("  - peer_id: {:?}", peer_id);
        log::trace!("  - connection_id: {:?}", connection_id);
        log::trace!("  - connected_point: {:?}", connected_point);
    }

    fn inject_event(&mut self, peer_id: PeerId, _connection: ConnectionId, event: HandlerOutEvent) {
        log::trace!("inject_event: peer_id={}: {:?}", peer_id, event);

        match event {
            HandlerOutEvent::PeerExchangeEstablished { peer_contact } => {
                self.events.push_back(NetworkBehaviourAction::GenerateEvent(
                    DiscoveryEvent::Established {
                        peer_id: peer_contact.public_key().clone().into_peer_id(),
                    },
                ));
            }
            HandlerOutEvent::ObservedAddresses { observed_addresses } => {
                let score = AddressScore::Infinite;
                for address in observed_addresses {
                    self.events
                        .push_back(NetworkBehaviourAction::ReportObservedAddr { address, score });
                }
            }
            HandlerOutEvent::Update => self.events.push_back(
                NetworkBehaviourAction::GenerateEvent(DiscoveryEvent::Update),
            ),
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<HandlerInEvent, DiscoveryEvent>> {
        // Emit events
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        // Poll house-keeping timer
        match self.house_keeping_timer.poll_next_unpin(cx) {
            Poll::Ready(Some(_)) => {
                log::debug!("Doing house-keeping in peer address book.");
                let mut peer_address_book = self.peer_contact_book.write();
                peer_address_book.self_update(&self.keypair);
                peer_address_book.house_keeping();
            }
            Poll::Ready(None) => unreachable!(),
            Poll::Pending => {}
        }

        Poll::Pending
    }
}
