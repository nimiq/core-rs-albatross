use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{Sink, SinkExt, StreamExt};
use instant::Instant;
use libp2p::{
    identity::Keypair,
    swarm::{
        ConnectionHandler, ConnectionHandlerEvent, ConnectionHandlerUpgrErr, KeepAlive,
        NegotiatedSubstream, SubstreamProtocol,
    },
    Multiaddr,
};
use parking_lot::RwLock;
use rand::{seq::IteratorRandom, thread_rng};
use thiserror::Error;
use wasm_timer::Interval;

use beserial::SerializingError;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;
use nimiq_utils::tagged_signing::TaggedKeypair;

use super::{
    behaviour::DiscoveryConfig,
    message_codec::{MessageReader, MessageWriter},
    peer_contacts::{PeerContactBook, SignedPeerContact},
    protocol::{ChallengeNonce, DiscoveryMessage, DiscoveryProtocol},
};

#[derive(Clone, Debug)]
pub enum HandlerInEvent {
    /// Peer address that got us a connection
    ConnectionAddress(Multiaddr),
    /// Address seen from peer
    ObservedAddress(Multiaddr),
}

#[derive(Clone, Debug)]
pub enum HandlerOutEvent {
    /// List of observed addresses for the peer
    ObservedAddresses {
        observed_addresses: Vec<Multiaddr>,
    },
    /// A peer discovery exchange protocol with a peer has finalized
    PeerExchangeEstablished {
        peer_address: Multiaddr,
        peer_contact: SignedPeerContact,
    },
    Update,
}

#[derive(Debug, Error)]
pub enum DiscoveryHandlerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] SerializingError),

    #[error("Unexpected message for state {state:?}: {message:?}")]
    UnexpectedMessage {
        state: HandlerState,
        message: DiscoveryMessage,
    },

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
    Signing(#[from] libp2p::identity::error::SigningError),

    #[error("Received too frequent updates: {}", .interval.as_secs())]
    TooFrequentUpdates { interval: Duration },

    #[error("Received update with too many peer contacts: {num_peer_contacts}")]
    UpdateLimitExceeded { num_peer_contacts: usize },
}

impl DiscoveryHandlerError {
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

pub struct DiscoveryHandler {
    /// Configuration for peer discovery.
    config: DiscoveryConfig,

    /// Identity keypair for this node.
    keypair: Keypair,

    /// The peer contact book
    peer_contact_book: Arc<RwLock<PeerContactBook>>,

    /// The peer address we're connected to (address that got us connected).
    peer_address: Option<Multiaddr>,

    /// The addresses which we observed for the other peer.
    observed_addresses: Vec<Multiaddr>,

    /// The challenge nonce we send to this peer.
    challenge_nonce: ChallengeNonce,

    /// Connection state
    state: HandlerState,

    /// Services filter sent to us by this peer.
    services_filter: Services,

    /// The limit for peer updates sent to us by this peer.
    peer_list_limit: Option<u16>,

    /// The interval at which the other peer wants to be updates.
    periodic_update_interval: Option<Interval>,

    /// Time when we last received an update from the other peer.
    last_update_time: Option<Instant>,

    /// The inbound message stream.
    inbound: Option<MessageReader<NegotiatedSubstream, DiscoveryMessage>>,

    /// The outbound message stream.
    outbound: Option<MessageWriter<NegotiatedSubstream, DiscoveryMessage>>,

    /// Waker used when opening a substream.
    waker: Option<Waker>,
}

impl DiscoveryHandler {
    pub fn new(
        config: DiscoveryConfig,
        keypair: Keypair,
        peer_contact_book: Arc<RwLock<PeerContactBook>>,
    ) -> Self {
        Self {
            config,
            keypair,
            peer_contact_book,
            peer_address: None,
            observed_addresses: vec![],
            challenge_nonce: ChallengeNonce::generate(),
            state: HandlerState::Init,
            services_filter: Services::empty(),
            peer_list_limit: None,
            periodic_update_interval: None,
            last_update_time: None,
            inbound: None,
            outbound: None,
            waker: None,
        }
    }

    fn send(&mut self, message: &DiscoveryMessage) -> Result<(), SerializingError> {
        Pin::new(self.outbound.as_mut().expect("Expected outbound substream")).start_send(message)
    }

    fn receive(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<Result<DiscoveryMessage, SerializingError>>> {
        self.inbound
            .as_mut()
            .expect("Expected inbound substream")
            .poll_next_unpin(cx)
    }

    /// Get peer contacts from our contact book to send to this peer. The contacts are filtered according to the peer's
    /// protocols and service filters, they are limited to the number of peers specified by the peer.
    fn get_peer_contacts(&self, peer_contact_book: &PeerContactBook) -> Vec<SignedPeerContact> {
        let n = self.peer_list_limit.unwrap() as usize;

        let mut rng = thread_rng();

        peer_contact_book
            .query(self.services_filter)
            .choose_multiple(&mut rng, n)
            .into_iter()
            .map(|c| c.signed().clone())
            .collect()
    }

    /// Checks if the handler is ready to start the discovery protocol.
    /// This basically checks that:
    /// - Both inbound and outbound are available
    /// - The connection peer address is already resolved.
    /// If these conditions are met, it transitions to sending a handshake and waking
    /// the waker.
    fn check_initialized(&mut self) {
        if self.inbound.is_some() && self.outbound.is_some() && self.peer_address.is_some() {
            debug!("Inbound and outbound connected. Performing handshake");

            self.state = HandlerState::SendHandshake;

            self.waker
                .take()
                .expect("Expected waker to be present")
                .wake();
        }
    }
}

impl ConnectionHandler for DiscoveryHandler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = DiscoveryHandlerError;
    type InboundProtocol = DiscoveryProtocol;
    type OutboundProtocol = DiscoveryProtocol;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<DiscoveryProtocol, ()> {
        SubstreamProtocol::new(DiscoveryProtocol, ())
    }

    fn inject_fully_negotiated_inbound(
        &mut self,
        protocol: MessageReader<NegotiatedSubstream, DiscoveryMessage>,
        _info: (),
    ) {
        if self.inbound.is_some() {
            panic!("Inbound already connected");
        }

        self.inbound = Some(protocol);

        self.check_initialized();
    }

    fn inject_fully_negotiated_outbound(
        &mut self,
        protocol: MessageWriter<NegotiatedSubstream, DiscoveryMessage>,
        _info: (),
    ) {
        if self.outbound.is_some() {
            panic!("Outbound already connected");
        }

        if self.state != HandlerState::OpenSubstream {
            panic!("Unexpected outbound");
        }

        self.outbound = Some(protocol);

        self.check_initialized();
    }

    fn inject_event(&mut self, event: HandlerInEvent) {
        match event {
            HandlerInEvent::ConnectionAddress(address) => {
                self.peer_address = Some(address);
                self.check_initialized();
            }
            HandlerInEvent::ObservedAddress(address) => {
                // We only use this during handshake and are not waiting on it, so we don't need to wake anything.
                self.observed_addresses.push(address);
            }
        }
    }

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        error: ConnectionHandlerUpgrErr<SerializingError>,
    ) {
        error!(%error, "inject_dial_upgrade_error");
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        self.config.keep_alive
    }

    fn poll(
        &mut self,
        cx: &mut Context,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, (), HandlerOutEvent, DiscoveryHandlerError>,
    > {
        loop {
            // Send message
            // This should be done first, so we can flush the outbound sink's buffer.
            if let Some(outbound) = self.outbound.as_mut() {
                match outbound.poll_ready_unpin(cx) {
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(ConnectionHandlerEvent::Close(e.into()))
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
                        observed_addresses: self.observed_addresses.clone(),
                        challenge_nonce: self.challenge_nonce.clone(),
                        genesis_hash: self.config.genesis_hash.clone(),
                        limit: self.config.update_limit,
                        services: self.config.required_services,
                        // TODO: If we really include this here, put this in `DiscoveryConfig`
                        user_agent: "TODO".to_string(),
                    };

                    if let Err(e) = self.send(&msg) {
                        return Poll::Ready(ConnectionHandlerEvent::Close(e.into()));
                    }

                    self.state = HandlerState::ReceiveHandshake;
                }

                HandlerState::ReceiveHandshake => {
                    // Wait for the peer's handshake message
                    match self.receive(cx) {
                        Poll::Ready(Some(Ok(message))) => {
                            match message {
                                DiscoveryMessage::Handshake {
                                    observed_addresses,
                                    challenge_nonce,
                                    genesis_hash,
                                    limit,
                                    services,
                                    user_agent: _,
                                } => {
                                    // Check if the received genesis hash matches.
                                    if genesis_hash != self.config.genesis_hash {
                                        return Poll::Ready(ConnectionHandlerEvent::Close(
                                            DiscoveryHandlerError::GenesisHashMismatch {
                                                expected: self.config.genesis_hash.clone(),
                                                received: genesis_hash,
                                            },
                                        ));
                                    }

                                    let mut peer_contact_book = self.peer_contact_book.write();

                                    // Update our own peer contact given the observed addresses we received
                                    peer_contact_book.add_own_addresses(
                                        observed_addresses.clone(),
                                        &self.keypair,
                                    );

                                    // Send the HandshakeAck
                                    let response_signature =
                                        self.keypair.tagged_sign(&challenge_nonce);

                                    // Remember peer's filter
                                    self.peer_list_limit = Some(limit);
                                    self.services_filter = services;

                                    let msg = DiscoveryMessage::HandshakeAck {
                                        peer_contact: peer_contact_book
                                            .get_own_contact()
                                            .signed()
                                            .clone(),
                                        response_signature,
                                        update_interval: Some(
                                            self.config.update_interval.as_secs(),
                                        ),
                                        peer_contacts: self.get_peer_contacts(&peer_contact_book),
                                    };

                                    drop(peer_contact_book);

                                    if let Err(e) = self.send(&msg) {
                                        return Poll::Ready(ConnectionHandlerEvent::Close(
                                            e.into(),
                                        ));
                                    }

                                    self.state = HandlerState::ReceiveHandshakeAck;

                                    return Poll::Ready(ConnectionHandlerEvent::Custom(
                                        HandlerOutEvent::ObservedAddresses { observed_addresses },
                                    ));
                                }

                                _ => {
                                    return Poll::Ready(ConnectionHandlerEvent::Close(
                                        DiscoveryHandlerError::UnexpectedMessage {
                                            message,
                                            state: self.state,
                                        },
                                    ))
                                }
                            }
                        }
                        Poll::Ready(None) => {
                            return Poll::Ready(ConnectionHandlerEvent::Close(
                                DiscoveryHandlerError::connection_reset(),
                            ))
                        }
                        Poll::Ready(Some(Err(e))) => {
                            return Poll::Ready(ConnectionHandlerEvent::Close(e.into()))
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
                                        return Poll::Ready(ConnectionHandlerEvent::Close(
                                            DiscoveryHandlerError::InvalidPeerContactSignature {
                                                peer_contact,
                                            },
                                        ));
                                    }

                                    // TODO: Do we need to check other stuff in the peer contact?

                                    // TODO: Check that the public key is actually used for this connection

                                    // Check the challenge response.
                                    if !response_signature.tagged_verify(
                                        &self.challenge_nonce,
                                        peer_contact.public_key(),
                                    ) {
                                        return Poll::Ready(ConnectionHandlerEvent::Close(
                                            DiscoveryHandlerError::ChallengeResponseFailed,
                                        ));
                                    }

                                    let mut peer_contact_book = self.peer_contact_book.write();

                                    // Insert the peer into the peer contact book.
                                    peer_contact_book.insert_filtered(
                                        peer_contact.clone(),
                                        self.config.required_services,
                                    );

                                    // Insert the peer's contacts (filtered) into my contact book
                                    peer_contact_book.insert_all_filtered(
                                        peer_contacts,
                                        self.config.required_services,
                                    );

                                    drop(peer_contact_book);

                                    // Timer for periodic updates
                                    if let Some(mut update_interval) = update_interval {
                                        let min_secs =
                                            self.config.min_send_update_interval.as_secs();
                                        if update_interval < min_secs {
                                            update_interval = min_secs;
                                        }
                                        self.periodic_update_interval = Some(Interval::new(
                                            Duration::from_secs(update_interval),
                                        ));
                                    }

                                    // Switch to established state
                                    self.state = HandlerState::Established;

                                    // Return an event that we established PEX with a new peer.
                                    return Poll::Ready(ConnectionHandlerEvent::Custom(
                                        HandlerOutEvent::PeerExchangeEstablished {
                                            peer_contact,
                                            peer_address: self
                                                .peer_address
                                                .clone()
                                                .expect("Address should have been resolved"),
                                        },
                                    ));
                                }

                                _ => {
                                    return Poll::Ready(ConnectionHandlerEvent::Close(
                                        DiscoveryHandlerError::UnexpectedMessage {
                                            message,
                                            state: self.state,
                                        },
                                    ))
                                }
                            }
                        }
                        Poll::Ready(None) => {
                            return Poll::Ready(ConnectionHandlerEvent::Close(
                                DiscoveryHandlerError::connection_reset(),
                            ))
                        }
                        Poll::Ready(Some(Err(e))) => {
                            return Poll::Ready(ConnectionHandlerEvent::Close(e.into()))
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
                                            return Poll::Ready(ConnectionHandlerEvent::Close(
                                                DiscoveryHandlerError::TooFrequentUpdates {
                                                    interval,
                                                },
                                            ));
                                        }
                                    }
                                    self.last_update_time = Some(now);

                                    // Check if the update is not too large.
                                    if peer_contacts.len() > self.config.update_limit as usize {
                                        return Poll::Ready(ConnectionHandlerEvent::Close(
                                            DiscoveryHandlerError::UpdateLimitExceeded {
                                                num_peer_contacts: peer_contacts.len(),
                                            },
                                        ));
                                    }

                                    // Insert the new peer contacts into the peer contact book.
                                    self.peer_contact_book.write().insert_all_filtered(
                                        peer_contacts,
                                        self.config.required_services,
                                    );

                                    return Poll::Ready(ConnectionHandlerEvent::Custom(
                                        HandlerOutEvent::Update,
                                    ));
                                }

                                _ => {
                                    return Poll::Ready(ConnectionHandlerEvent::Close(
                                        DiscoveryHandlerError::UnexpectedMessage {
                                            message,
                                            state: self.state,
                                        },
                                    ))
                                }
                            }
                        }
                        Poll::Ready(None) => {
                            return Poll::Ready(ConnectionHandlerEvent::Close(
                                DiscoveryHandlerError::connection_reset(),
                            ))
                        }
                        Poll::Ready(Some(Err(e))) => {
                            return Poll::Ready(ConnectionHandlerEvent::Close(e.into()))
                        }
                        Poll::Pending => {}
                    }

                    // Periodically send out updates.
                    if let Some(timer) = self.periodic_update_interval.as_mut() {
                        match timer.poll_next_unpin(cx) {
                            Poll::Ready(Some(_instant)) => {
                                let peer_contacts =
                                    self.get_peer_contacts(&self.peer_contact_book.read());

                                if !peer_contacts.is_empty() {
                                    let msg = DiscoveryMessage::PeerAddresses { peer_contacts };

                                    if let Err(e) = self.send(&msg) {
                                        return Poll::Ready(ConnectionHandlerEvent::Close(
                                            e.into(),
                                        ));
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

        // If we've left the loop, we're waiting on something.
        Poll::Pending
    }
}
