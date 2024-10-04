use std::{
    collections::{HashSet, VecDeque},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{FutureExt, Sink, SinkExt, StreamExt};
use futures_timer::Delay;
use instant::Instant;
use libp2p::{
    identity::Keypair,
    multiaddr::Protocol,
    swarm::{
        handler::{
            ConnectionEvent, DialUpgradeError, FullyNegotiatedInbound, FullyNegotiatedOutbound,
            ProtocolSupport,
        },
        ConnectionHandler, ConnectionHandlerEvent, Stream, SubstreamProtocol,
    },
    Multiaddr, PeerId, StreamProtocol,
};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;
use nimiq_serde::DeserializeError;
use nimiq_time::{interval, Interval};
use nimiq_utils::tagged_signing::TaggedKeyPair;
use parking_lot::RwLock;
use rand::{seq::IteratorRandom, thread_rng};
use thiserror::Error;

use super::{
    behaviour::Config,
    message_codec::{MessageReader, MessageWriter},
    peer_contacts::{PeerContactBook, SignedPeerContact},
    protocol::{ChallengeNonce, DiscoveryMessage, DiscoveryProtocol},
};
use crate::{AUTONAT_DIAL_BACK_PROTOCOL, AUTONAT_DIAL_REQUEST_PROTOCOL};

#[derive(Debug)]
pub enum HandlerOutEvent {
    /// List of observed addresses for the peer
    ObservedAddress {
        observed_address: Multiaddr,
    },
    /// A peer discovery exchange protocol with a peer has finalized
    PeerExchangeEstablished {
        peer_address: Multiaddr,
        peer_contact: SignedPeerContact,
    },
    Update,
    /// An error occurred
    Error(Error),
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] DeserializeError),

    #[error("Unexpected message for state {state:?}: {message:?}")]
    UnexpectedMessage {
        state: HandlerState,
        message: DiscoveryMessage,
    },

    #[error("Timeout reached for {state:?}: peer might not be responding")]
    StateTransitionTimeout { state: HandlerState },

    #[error("Mismatch for genesis hash: Expected {expected}, but received {received}")]
    GenesisHashMismatch {
        expected: Blake2bHash,
        received: Blake2bHash,
    },

    #[error("Peer contact has an invalid signature: {peer_contact:?}")]
    InvalidPeerContactSignature { peer_contact: SignedPeerContact },

    #[error("Peer replied with incorrect response to challenge.")]
    ChallengeResponseFailed,

    #[error("Signing error: {0}")]
    Signing(#[from] libp2p::identity::SigningError),

    #[error("Received too frequent updates: {}", .interval.as_secs())]
    TooFrequentUpdates { interval: Duration },

    #[error("Received update with too many peer contacts: {num_peer_contacts}")]
    UpdateLimitExceeded { num_peer_contacts: usize },
}

impl Error {
    /// Short-hand to create an IO error variant with an ConnectionReset error.
    pub fn connection_reset() -> Self {
        Self::Io(std::io::ErrorKind::ConnectionReset.into())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HandlerState {
    /// The handler needs to be initialized, i.e. and outbound substream has to be opened.
    Init,

    /// Wait for the substream to be opened.
    OpenSubstream,

    /// Send a handshake.
    SendHandshake,

    /// Receive handshake from peer and send a HandshakeAck.
    ReceiveHandshake,

    /// Receive HandshakeAck from other peer.
    ReceiveHandshakeAck,

    /// Peer exchange is established. We now will send peer lists periodically.
    Established,
}

pub struct Handler {
    /// Peer ID of the peer connected to us in this connection
    peer_id: PeerId,

    /// Configuration for peer discovery.
    config: Config,

    /// Identity keypair for this node.
    keypair: Keypair,

    /// The peer contact book
    peer_contact_book: Arc<RwLock<PeerContactBook>>,

    /// The peer address we're connected to (address that got us connected).
    peer_address: Multiaddr,

    /// The challenge nonce we send to this peer.
    challenge_nonce: ChallengeNonce,

    /// Connection state
    state: HandlerState,

    /// Future that fires on a state change timeout
    state_timeout: Option<Delay>,

    /// Services filter sent to us by this peer.
    services_filter: Services,

    /// The limit for peer updates sent to us by this peer.
    peer_list_limit: Option<u16>,

    /// The interval at which the other peer wants to be updates.
    periodic_update_interval: Option<Interval>,

    /// Time when we last received an update from the other peer.
    last_update_time: Option<Instant>,

    /// The inbound message stream.
    inbound: Option<MessageReader<Stream, DiscoveryMessage>>,

    /// The outbound message stream.
    outbound: Option<MessageWriter<Stream, DiscoveryMessage>>,

    /// Waker used when opening a substream.
    waker: Option<Waker>,

    /// Events to inform its behaviour or all other ConnectionHandlers
    events: VecDeque<ConnectionHandlerEvent<DiscoveryProtocol, (), HandlerOutEvent>>,
}

impl Handler {
    const STATE_TRANSITION_TIMEOUT: Duration = Duration::from_millis(3000);
    pub fn new(
        peer_id: PeerId,
        config: Config,
        keypair: Keypair,
        peer_contact_book: Arc<RwLock<PeerContactBook>>,
        peer_address: Multiaddr,
    ) -> Self {
        if let Some(peer_contact) = peer_contact_book.write().get(&peer_id) {
            if let Some(outer_protocol_address) = outer_protocol_address(&peer_address) {
                peer_contact.set_outer_protocol_address(outer_protocol_address);
            }
        }

        Self {
            peer_id,
            config,
            keypair,
            peer_contact_book,
            peer_address,
            challenge_nonce: ChallengeNonce::generate(),
            state: HandlerState::Init,
            state_timeout: None,
            services_filter: Services::empty(),
            peer_list_limit: None,
            periodic_update_interval: None,
            last_update_time: None,
            inbound: None,
            outbound: None,
            waker: None,
            events: VecDeque::new(),
        }
    }

    fn send(&mut self, message: &DiscoveryMessage) -> Result<(), std::io::Error> {
        Pin::new(self.outbound.as_mut().expect("Expected outbound substream")).start_send(message)
    }

    fn receive(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<Result<DiscoveryMessage, DeserializeError>>> {
        self.inbound
            .as_mut()
            .expect("Expected inbound substream")
            .poll_next_unpin(cx)
    }

    /// Get peer contacts from our contact book to send to this peer. The contacts are filtered according to the peer's
    /// protocols and service filters, they are limited to the number of peers specified by the peer.
    /// This list also includes our own contact which should be already filtered since we already have
    /// connections to these peers.
    fn get_peer_contacts(
        &self,
        peer_contact_book: &PeerContactBook,
        limit: usize,
    ) -> Vec<SignedPeerContact> {
        let mut rng = thread_rng();

        peer_contact_book
            .query(self.services_filter)
            .choose_multiple(&mut rng, limit)
            .into_iter()
            .map(|c| c.signed().clone())
            .collect()
    }

    /// Checks if the handler is ready to start the discovery protocol.
    /// This basically checks that:
    /// - Both inbound and outbound are available
    ///
    /// If these conditions are met, it transitions to sending a handshake and waking
    /// the waker.
    fn check_initialized(&mut self) {
        if self.inbound.is_some() && self.outbound.is_some() {
            self.state = HandlerState::SendHandshake;
            self.state_timeout = Some(Delay::new(Self::STATE_TRANSITION_TIMEOUT));

            self.waker
                .take()
                .expect("Expected waker to be present")
                .wake();
        }
    }

    /// Report to all the ConnectionHandlers that the remote peer supports the AutoNAT V2 client and server protocols
    fn report_remote_autonat_support(&mut self) {
        let mut stream_protocols = HashSet::new();
        stream_protocols.insert(StreamProtocol::new(AUTONAT_DIAL_REQUEST_PROTOCOL));
        stream_protocols.insert(StreamProtocol::new(AUTONAT_DIAL_BACK_PROTOCOL));

        self.events
            .push_back(ConnectionHandlerEvent::ReportRemoteProtocols(
                ProtocolSupport::Added(stream_protocols),
            ));
    }
}

/// Extract the `/ip4/`,`/ip6/`, `/dns4/` or `/dns6/` protocol part from a `Multiaddr`
pub(crate) fn outer_protocol_address(addr: &Multiaddr) -> Option<Multiaddr> {
    addr.iter()
        .find(|p| {
            matches!(
                p,
                Protocol::Dns(_)
                    | Protocol::Dns4(_)
                    | Protocol::Dns6(_)
                    | Protocol::Ip4(_)
                    | Protocol::Ip6(_)
            )
        })
        .map(|p| Some(p.into()))
        .unwrap_or(None)
}

impl ConnectionHandler for Handler {
    type FromBehaviour = ();
    type ToBehaviour = HandlerOutEvent;
    type InboundProtocol = DiscoveryProtocol;
    type OutboundProtocol = DiscoveryProtocol;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<DiscoveryProtocol, ()> {
        SubstreamProtocol::new(DiscoveryProtocol, ())
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol, ..
            }) => {
                if self.inbound.is_some() {
                    panic!("Inbound already connected");
                }
                self.inbound = Some(protocol);
                self.check_initialized();
                self.report_remote_autonat_support();
            }
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol, ..
            }) => {
                if self.outbound.is_some() {
                    panic!("Outbound already connected");
                }
                if self.state != HandlerState::OpenSubstream {
                    panic!("Unexpected outbound");
                }
                self.outbound = Some(protocol);
                self.check_initialized();
                self.report_remote_autonat_support();
            }
            ConnectionEvent::DialUpgradeError(DialUpgradeError { error, .. }) => {
                error!(%error, "inject_dial_upgrade_error");
            }
            _ => {}
        }
    }

    fn on_behaviour_event(&mut self, _event: ()) {}

    fn connection_keep_alive(&self) -> bool {
        self.config.keep_alive
    }

    fn poll(
        &mut self,
        cx: &mut Context,
    ) -> Poll<ConnectionHandlerEvent<Self::OutboundProtocol, (), HandlerOutEvent>> {
        loop {
            // Check if we hit the state transition timeout
            if let Some(ref mut state_timeout) = self.state_timeout {
                if state_timeout.poll_unpin(cx).is_ready() {
                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        HandlerOutEvent::Error(Error::StateTransitionTimeout { state: self.state }),
                    ));
                }
            }

            // Send message
            // This should be done first, so we can flush the outbound sink's buffer.
            if let Some(outbound) = self.outbound.as_mut() {
                match outbound.poll_ready_unpin(cx) {
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                            HandlerOutEvent::Error(e.into()),
                        ))
                    }

                    // Make sure the outbound sink is ready before we continue.
                    Poll::Pending => break,

                    // Outbound sink is ready, so continue to state handling.
                    Poll::Ready(Ok(())) => {}
                }
            }

            // Handle state
            match self.state {
                HandlerState::Init => {
                    // Request outbound substream
                    self.state = HandlerState::OpenSubstream;
                    self.state_timeout = Some(Delay::new(Self::STATE_TRANSITION_TIMEOUT));

                    return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                        protocol: SubstreamProtocol::new(DiscoveryProtocol, ()),
                    });
                }

                HandlerState::OpenSubstream => {
                    // Wait for the substream to be opened
                    if self.waker.is_none() {
                        self.waker = Some(cx.waker().clone());
                    }

                    // We need to wait for the inbound substream to be injected.
                    break;
                }

                HandlerState::SendHandshake => {
                    // Send out a handshake message.

                    let msg = DiscoveryMessage::Handshake {
                        observed_address: self.peer_address.clone(),
                        challenge_nonce: self.challenge_nonce.clone(),
                        genesis_hash: self.config.genesis_hash.clone(),
                        limit: self.config.update_limit,
                        services: self.config.required_services,
                    };

                    if let Err(e) = self.send(&msg) {
                        return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                            HandlerOutEvent::Error(e.into()),
                        ));
                    }

                    self.state = HandlerState::ReceiveHandshake;
                    self.state_timeout = Some(Delay::new(Self::STATE_TRANSITION_TIMEOUT));
                }

                HandlerState::ReceiveHandshake => {
                    // Wait for the peer's handshake message
                    match self.receive(cx) {
                        Poll::Ready(Some(Ok(message))) => {
                            match message {
                                DiscoveryMessage::Handshake {
                                    observed_address,
                                    challenge_nonce,
                                    genesis_hash,
                                    limit,
                                    services,
                                } => {
                                    // Check if the received genesis hash matches.
                                    if genesis_hash != self.config.genesis_hash {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(
                                                    Error::GenesisHashMismatch {
                                                        expected: self.config.genesis_hash.clone(),
                                                        received: genesis_hash,
                                                    },
                                                ),
                                            ),
                                        );
                                    }

                                    // Send the HandshakeAck
                                    let response_signature =
                                        self.keypair.tagged_sign(&challenge_nonce);

                                    // Remember peer's filter
                                    self.peer_list_limit = Some(limit);
                                    self.services_filter = services;

                                    let peer_contact_book = self.peer_contact_book.read();

                                    let msg = DiscoveryMessage::HandshakeAck {
                                        peer_contact: peer_contact_book
                                            .get_own_contact()
                                            .signed()
                                            .clone(),
                                        response_signature,
                                        update_interval: Some(
                                            self.config.update_interval.as_secs(),
                                        ),
                                        peer_contacts: self.get_peer_contacts(
                                            &peer_contact_book,
                                            self.peer_list_limit.unwrap() as usize,
                                        ),
                                    };

                                    drop(peer_contact_book);

                                    if let Err(e) = self.send(&msg) {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(e.into()),
                                            ),
                                        );
                                    }

                                    self.state = HandlerState::ReceiveHandshakeAck;
                                    self.state_timeout =
                                        Some(Delay::new(Self::STATE_TRANSITION_TIMEOUT));

                                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                        HandlerOutEvent::ObservedAddress { observed_address },
                                    ));
                                }

                                _ => {
                                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                        HandlerOutEvent::Error(Error::UnexpectedMessage {
                                            message,
                                            state: self.state,
                                        }),
                                    ))
                                }
                            }
                        }
                        Poll::Ready(None) => {
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HandlerOutEvent::Error(Error::connection_reset()),
                            ))
                        }
                        Poll::Ready(Some(Err(e))) => {
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HandlerOutEvent::Error(e.into()),
                            ))
                        }
                        Poll::Pending => break,
                    }
                }

                HandlerState::ReceiveHandshakeAck => {
                    // Wait for the peer's HandshakeAck message
                    match self.receive(cx) {
                        Poll::Ready(Some(Ok(message))) => {
                            match message {
                                DiscoveryMessage::HandshakeAck {
                                    peer_contact,
                                    response_signature,
                                    update_interval,
                                    peer_contacts,
                                } => {
                                    // Check the peer contact for a valid signature.
                                    if !peer_contact.verify() {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(
                                                    Error::InvalidPeerContactSignature {
                                                        peer_contact,
                                                    },
                                                ),
                                            ),
                                        );
                                    }

                                    if self.peer_id != peer_contact.peer_id() {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(
                                                    Error::ChallengeResponseFailed,
                                                ),
                                            ),
                                        );
                                    }

                                    // Check the challenge response.
                                    if !response_signature.tagged_verify(
                                        &self.challenge_nonce,
                                        peer_contact.public_key(),
                                    ) {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(
                                                    Error::ChallengeResponseFailed,
                                                ),
                                            ),
                                        );
                                    }

                                    // Check and verify the peer contacts received
                                    if peer_contacts.len() > self.config.update_limit as usize {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(
                                                    Error::UpdateLimitExceeded {
                                                        num_peer_contacts: peer_contacts.len(),
                                                    },
                                                ),
                                            ),
                                        );
                                    }
                                    for peer_contact in &peer_contacts {
                                        if !peer_contact.verify() {
                                            return Poll::Ready(
                                                ConnectionHandlerEvent::NotifyBehaviour(
                                                    HandlerOutEvent::Error(
                                                        Error::InvalidPeerContactSignature {
                                                            peer_contact: peer_contact.clone(),
                                                        },
                                                    ),
                                                ),
                                            );
                                        }
                                    }

                                    let mut peer_contact_book = self.peer_contact_book.write();

                                    // Insert the peer into the peer contact book.
                                    peer_contact_book.insert(peer_contact.clone());

                                    // Insert the peer's contacts (filtered) into my contact book
                                    peer_contact_book.insert_all_filtered(
                                        peer_contacts,
                                        self.config.required_services,
                                        self.config.only_secure_ws_connections,
                                    );

                                    drop(peer_contact_book);

                                    // Timer for periodic updates
                                    if let Some(mut update_interval) = update_interval {
                                        let min_secs =
                                            self.config.min_send_update_interval.as_secs();
                                        if update_interval < min_secs {
                                            update_interval = min_secs;
                                        }
                                        self.periodic_update_interval =
                                            Some(interval(Duration::from_secs(update_interval)));
                                    }

                                    // Switch to established state
                                    self.state = HandlerState::Established;
                                    self.state_timeout = None;

                                    // Return an event that we established PEX with a new peer.
                                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                        HandlerOutEvent::PeerExchangeEstablished {
                                            peer_contact,
                                            peer_address: self.peer_address.clone(),
                                        },
                                    ));
                                }

                                _ => {
                                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                        HandlerOutEvent::Error(Error::UnexpectedMessage {
                                            message,
                                            state: self.state,
                                        }),
                                    ))
                                }
                            }
                        }
                        Poll::Ready(None) => {
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HandlerOutEvent::Error(Error::connection_reset()),
                            ))
                        }
                        Poll::Ready(Some(Err(e))) => {
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HandlerOutEvent::Error(e.into()),
                            ))
                        }
                        Poll::Pending => break,
                    }
                }

                HandlerState::Established => {
                    // Check for incoming updates.
                    match self.receive(cx) {
                        Poll::Ready(Some(Ok(message))) => {
                            match message {
                                DiscoveryMessage::PeerAddresses { peer_contacts } => {
                                    // Check if the update is actually not too frequent
                                    let now = Instant::now();
                                    if let Some(last_update_time) = self.last_update_time {
                                        let interval = now - last_update_time;
                                        if interval < self.config.min_recv_update_interval {
                                            // TODO: Should we just close, or ban?
                                            return Poll::Ready(
                                                ConnectionHandlerEvent::NotifyBehaviour(
                                                    HandlerOutEvent::Error(
                                                        Error::TooFrequentUpdates { interval },
                                                    ),
                                                ),
                                            );
                                        }
                                    }
                                    self.last_update_time = Some(now);

                                    // Check if the update is not too large and if the peer contacts verify
                                    if peer_contacts.len() > self.config.update_limit as usize {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(
                                                    Error::UpdateLimitExceeded {
                                                        num_peer_contacts: peer_contacts.len(),
                                                    },
                                                ),
                                            ),
                                        );
                                    }
                                    for peer_contact in &peer_contacts {
                                        if !peer_contact.verify() {
                                            return Poll::Ready(
                                                ConnectionHandlerEvent::NotifyBehaviour(
                                                    HandlerOutEvent::Error(
                                                        Error::InvalidPeerContactSignature {
                                                            peer_contact: peer_contact.clone(),
                                                        },
                                                    ),
                                                ),
                                            );
                                        }
                                    }

                                    // Insert the new peer contacts into the peer contact book.
                                    self.peer_contact_book.write().insert_all_filtered(
                                        peer_contacts,
                                        self.config.required_services,
                                        self.config.only_secure_ws_connections,
                                    );

                                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                        HandlerOutEvent::Update,
                                    ));
                                }

                                _ => {
                                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                        HandlerOutEvent::Error(Error::UnexpectedMessage {
                                            message,
                                            state: self.state,
                                        }),
                                    ))
                                }
                            }
                        }
                        Poll::Ready(None) => {
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HandlerOutEvent::Error(Error::connection_reset()),
                            ))
                        }
                        Poll::Ready(Some(Err(e))) => {
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HandlerOutEvent::Error(e.into()),
                            ))
                        }
                        Poll::Pending => {}
                    }

                    // Periodically send out updates.
                    if let Some(timer) = self.periodic_update_interval.as_mut() {
                        match timer.poll_next_unpin(cx) {
                            Poll::Ready(Some(_instant)) => {
                                let peer_contacts = {
                                    let peer_contact_book = &self.peer_contact_book.read();
                                    let mut peer_contacts = self.get_peer_contacts(
                                        peer_contact_book,
                                        self.peer_list_limit.unwrap() as usize - 1,
                                    );
                                    // Always include our own contact for updates
                                    peer_contacts
                                        .push(peer_contact_book.get_own_contact().signed().clone());
                                    peer_contacts
                                };

                                if !peer_contacts.is_empty() {
                                    let msg = DiscoveryMessage::PeerAddresses { peer_contacts };

                                    if let Err(e) = self.send(&msg) {
                                        return Poll::Ready(
                                            ConnectionHandlerEvent::NotifyBehaviour(
                                                HandlerOutEvent::Error(e.into()),
                                            ),
                                        );
                                    }
                                }
                            }
                            Poll::Ready(None) => unreachable!("Interval terminated"),
                            Poll::Pending => break,
                        }
                    }
                }
            }
        }

        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        // If we've left the loop, we're waiting on something.
        Poll::Pending
    }
}
