use std::{
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use libp2p::{
    swarm::{
        ProtocolsHandler, SubstreamProtocol, ProtocolsHandlerUpgrErr, KeepAlive, ProtocolsHandlerEvent,
        NegotiatedSubstream,
    },
    identity::Keypair,
    Multiaddr,
};
use futures::{
    task::{Context, Poll, Waker},
    Stream, StreamExt, Sink, SinkExt, ready,
};
use parking_lot::RwLock;
use thiserror::Error;
use wasm_timer::Interval;
use rand::{
    seq::IteratorRandom,
    thread_rng,
};

use beserial::SerializingError;
use nimiq_hash::Blake2bHash;

use super::{
    behaviour::DiscoveryConfig,
    protocol::{DiscoveryProtocol, DiscoveryMessage, ChallengeNonce},
    peer_contacts::{Services, Protocols, PeerContactBook, SignedPeerContact},
};
use crate::{
    message::{MessageReader, MessageWriter},
    tagged_signing::{TaggedKeypair, TaggedPublicKey}
};


#[derive(Clone, Debug)]
pub enum HandlerInEvent {
    ObservedAddress(Multiaddr),
}


#[derive(Clone, Debug)]
pub enum HandlerOutEvent {
    ObservedAddresses {
        observed_addresses: Vec<Multiaddr>,
    },
    PeerExchangeEstablished {
        peer_contact: SignedPeerContact,
    },
    Update,
}


#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] SerializingError),

    #[error("Unexpected message for state {state:?}: {message:?}")]
    UnexpectedMessage {
        state: HandlerState,
        message: DiscoveryMessage
    },

    #[error("Mismatch for genesis hash: Expected {expected}, but received {received}")]
    GenesisHashMismatch {
        expected: Blake2bHash,
        received: Blake2bHash,
    },

    #[error("Peer contact has an invalid signature: {peer_contact:?}")]
    InvalidPeerContactSignature {
        peer_contact: SignedPeerContact,
    },

    #[error("Peer replied with incorrect response to challenge.")]
    ChallengeResponseFailed,

    #[error("Signing error: {0}")]
    Signing(#[from] libp2p::identity::error::SigningError),

    #[error("Received too frequent updates: {}", .interval.as_secs())]
    TooFrequentUpdates {
        interval: Duration
    },

    #[error("Received update with too many peer contacts: {num_peer_contacts}")]
    UpdateLimitExceeded {
        num_peer_contacts: usize,
    },
}

impl HandlerError {
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

    /// The peer contact of the peer we're connected to.
    peer_contact: Option<SignedPeerContact>,

    /// The addresses which we observed for the other peer.
    observed_addresses: Vec<Multiaddr>,

    /// The challenge nonce we send to this peer.
    challenge_nonce: ChallengeNonce,

    /// Connection state
    state: HandlerState,

    /// Services filter sent to us by this peer.
    services_filter: Services,

    /// Protocols filter sent to us by this peer.
    protocols_filter: Protocols,

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
    pub fn new(config: DiscoveryConfig, keypair: Keypair, peer_contact_book: Arc<RwLock<PeerContactBook>>) -> Self {
        Self {
            config,
            keypair,
            peer_contact_book,
            peer_contact: None,
            observed_addresses: vec![],
            challenge_nonce: ChallengeNonce::generate(),
            state: HandlerState::Init,
            services_filter: Services::empty(),
            protocols_filter: Protocols::empty(),
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

    fn receive(&mut self, cx: &mut Context) -> Poll<Option<Result<DiscoveryMessage, SerializingError>>> {
        self.inbound.as_mut().expect("Expected inbound substream").poll_next_unpin(cx)
    }

    fn get_peer_contacts(&self, peer_contact_book: &PeerContactBook) -> Vec<SignedPeerContact> {
        let n = self.peer_list_limit.map(|l| l as usize).unwrap_or(128);

        let mut rng = thread_rng();

        peer_contact_book
            .query(self.protocols_filter, self.services_filter)
            .choose_multiple(&mut rng, n)
            .into_iter()
            .map(|c| c.signed().clone())
            .collect()
    }
}

impl ProtocolsHandler for DiscoveryHandler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = HandlerError;
    type InboundProtocol = DiscoveryProtocol;
    type OutboundProtocol = DiscoveryProtocol;
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol> {
        log::debug!("DiscoveryHandler::listen_protocol");
        SubstreamProtocol::new(DiscoveryProtocol)
    }

    fn inject_fully_negotiated_inbound(&mut self, protocol: MessageReader<NegotiatedSubstream, DiscoveryMessage>) {
        log::debug!("DiscoveryHandler::inject_fully_negotiated_inbound");

        if self.inbound.is_some() {
            panic!("Inbound already connected");
        }

        self.inbound = Some(protocol);
    }

    fn inject_fully_negotiated_outbound(&mut self, protocol: MessageWriter<NegotiatedSubstream, DiscoveryMessage>, info: ()) {
        log::debug!("DiscoveryHandler::inject_fully_negotiated_outbound");

        if self.outbound.is_some() {
            panic!("Outbound already connected");
        }

        if self.state != HandlerState::OpenSubstream {
            panic!("Unexpected outbound");
        }

        self.outbound = Some(protocol);
        self.state = HandlerState::SendHandshake;

        self.waker
            .take()
            .expect("Expected waker to be present")
            .wake();
    }

    fn inject_event(&mut self, event: HandlerInEvent) {
        log::debug!("DiscoveryHandler::inject_event: {:?}", event);

        match event {
            HandlerInEvent::ObservedAddress(address) => {
                // We only use this during handshake and are not waiting on it, so we don't need to wake anything.
                self.observed_addresses.push(address);
            }
        }
    }

    fn inject_dial_upgrade_error(&mut self, info: Self::OutboundOpenInfo, error: ProtocolsHandlerUpgrErr<SerializingError>) {
        log::warn!("DiscoveryHandler::inject_dial_upgrade_error: {:?}", error);
        unimplemented!()
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<ProtocolsHandlerEvent<Self::OutboundProtocol, (), HandlerOutEvent, HandlerError>> {
        loop {
            // Send message
            // This should be done first, so we can flush the outbound sink's buffer.
            if let Some(outbound) = self.outbound.as_mut() {
                log::trace!("DiscoveryHandler: Polling sink");
                match outbound.poll_ready_unpin(cx) {
                    Poll::Ready(Err(e)) => {
                        panic!("poll_ready failed: {}", e);
                        return Poll::Ready(ProtocolsHandlerEvent::Close(e.into()))
                    },

                    // Make sure the outbound sink is ready before we continue.
                    Poll::Pending => break,

                    // Outbound sink is ready, so continue to state handling.
                    Poll::Ready(Ok(())) => {},
                }
            }

            // Handle state
            log::trace!("DiscoveryHandler: state: {:?}", self.state);
            match self.state {
                HandlerState::Init => {
                    // Request outbound substream
                    self.state = HandlerState::OpenSubstream;

                    return Poll::Ready(ProtocolsHandlerEvent::OutboundSubstreamRequest {
                        protocol: SubstreamProtocol::new(DiscoveryProtocol),
                        info: ()
                    })
                },

                HandlerState::OpenSubstream => {
                    // Wait for the substream to be opened
                    if self.waker.is_none() {
                        self.waker = Some(cx.waker().clone());
                    }

                    // We need to wait for the inbound substream to be injected.
                    break;
                },

                HandlerState::SendHandshake => {
                    // Send out a handshake message.

                    let msg = DiscoveryMessage::Handshake {
                        observed_addresses: self.observed_addresses.clone(),
                        challenge_nonce: self.challenge_nonce.clone(),
                        genesis_hash: self.config.genesis_hash.clone(),
                        limit: self.config.update_limit,
                        services: self.config.services_filter,
                        protocols: self.config.protocols_filter,
                        // TODO: If we really include this here, put this in `DiscoveryConfig`
                        user_agent: "TODO".to_string(),
                    };

                    if let Err(e) = self.send(&msg) {
                        return Poll::Ready(ProtocolsHandlerEvent::Close(e.into()));
                    }

                    self.state = HandlerState::ReceiveHandshake;
                },

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
                                    protocols,
                                    user_agent: _,
                                } => {
                                    let mut peer_contact_book = self.peer_contact_book.write();

                                    // Check if the received genesis hash matches.
                                    if genesis_hash != self.config.genesis_hash {
                                        return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::GenesisHashMismatch {
                                            expected: self.config.genesis_hash.clone(),
                                            received: genesis_hash,
                                        }))
                                    }

                                    // Update our own peer contact given the observed addresses we received
                                    peer_contact_book.self_add_addresses(observed_addresses.clone());

                                    // Send the HandshakeAck
                                    let response_signature = self.keypair.tagged_sign(&challenge_nonce);

                                    // Remember peer's filter
                                    self.peer_list_limit = limit;
                                    self.services_filter = services;
                                    self.protocols_filter = protocols;

                                    let msg = DiscoveryMessage::HandshakeAck {
                                        peer_contact: peer_contact_book.get_self().signed().clone(),
                                        response_signature,
                                        update_interval: Some(self.config.update_interval.as_secs()),
                                        peer_contacts: self.get_peer_contacts(&peer_contact_book),
                                    };

                                    drop(peer_contact_book);

                                    if let Err(e) = self.send(&msg) {
                                        return Poll::Ready(ProtocolsHandlerEvent::Close(e.into()));
                                    }

                                    self.state = HandlerState::ReceiveHandshakeAck;

                                    return Poll::Ready(ProtocolsHandlerEvent::Custom(HandlerOutEvent::ObservedAddresses { observed_addresses }))
                                },

                                _ => return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::UnexpectedMessage {
                                    message,
                                    state: self.state,
                                })),
                            }
                        },
                        Poll::Ready(None) => return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::connection_reset())),
                        Poll::Ready(Some(Err(e))) => return Poll::Ready(ProtocolsHandlerEvent::Close(e.into())),
                        Poll::Pending => break,
                    }
                },

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
                                        return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::InvalidPeerContactSignature { peer_contact }))
                                    }

                                    // TODO: Do we need to check other stuff in the peer contact?

                                    // Check the challenge response.
                                    if !peer_contact.public_key().tagged_verify(&self.challenge_nonce, &response_signature) {
                                        return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::ChallengeResponseFailed))
                                    }

                                    let mut peer_contact_book = self.peer_contact_book.write();

                                    // Insert the peer into the peer contact book.
                                    peer_contact_book.insert_filtered(peer_contact.clone(), self.config.protocols_filter, self.config.services_filter);

                                    // Insert the initial set of peer contacts into the peer contact book.
                                    // TODO: This doesn't actually filter and just assumes the peer already filtered.
                                    peer_contact_book.insert_all(peer_contacts);

                                    // Store peer contact in handler
                                    self.peer_contact = Some(peer_contact.clone());

                                    // Timer for periodic updates
                                    if let Some(mut update_interval) = update_interval {
                                        log::debug!("Update interval: {:?}", update_interval);

                                        let min_secs = self.config.min_send_update_interval.as_secs();
                                        if update_interval < min_secs {
                                            update_interval = min_secs;
                                        }
                                        self.periodic_update_interval = Some(Interval::new(Duration::from_secs(update_interval)));
                                    }

                                    // Switch to established state
                                    self.state = HandlerState::Established;

                                    // TODO: Return an event that we established PEX with a new peer.
                                    return Poll::Ready(ProtocolsHandlerEvent::Custom(HandlerOutEvent::PeerExchangeEstablished {
                                        peer_contact,
                                    }))
                                },

                                _ => return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::UnexpectedMessage {
                                    message,
                                    state: self.state,
                                })),
                            }
                        },
                        Poll::Ready(None) => return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::connection_reset())),
                        Poll::Ready(Some(Err(e))) => return Poll::Ready(ProtocolsHandlerEvent::Close(e.into())),
                        Poll::Pending => break,
                    }
                },

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
                                            return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::TooFrequentUpdates { interval }));
                                        }
                                    }
                                    self.last_update_time = Some(now);

                                    // Check if the update is not too large.
                                    if let Some(update_limit) = self.config.update_limit {
                                        if peer_contacts.len() > update_limit as usize {
                                            return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::UpdateLimitExceeded { num_peer_contacts: peer_contacts.len() }));
                                        }
                                    }

                                    // Insert the new peer contacts into the peer contact book.
                                    self.peer_contact_book.write()
                                        .insert_all_filtered(
                                            peer_contacts,
                                            self.config.protocols_filter,
                                            self.config.services_filter
                                        );

                                    return Poll::Ready(ProtocolsHandlerEvent::Custom(HandlerOutEvent::Update))
                                },

                                _ => return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::UnexpectedMessage {
                                    message,
                                    state: self.state,
                                })),
                            }
                        },
                        Poll::Ready(None) => return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::connection_reset())),
                        Poll::Ready(Some(Err(e))) => return Poll::Ready(ProtocolsHandlerEvent::Close(e.into())),
                        Poll::Pending => {},
                    }

                    // Periodically send out updates.
                    if let Some(timer) = self.periodic_update_interval.as_mut() {
                        match timer.poll_next_unpin(cx) {
                            Poll::Ready(Some(_instant)) => {
                                let msg = DiscoveryMessage::PeerAddresses {
                                    peer_contacts: self.get_peer_contacts(&self.peer_contact_book.read()),
                                };

                                if let Err(e) = self.send(&msg) {
                                    return Poll::Ready(ProtocolsHandlerEvent::Close(e.into()));
                                }
                            },
                            Poll::Ready(None) => todo!("Interval terminated"),
                            Poll::Pending => break,
                        }
                    }
                },
            }
        }

        // If we've left the loop, we're waiting on something.
        Poll::Pending
    }
}
