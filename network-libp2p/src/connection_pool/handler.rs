use std::collections::VecDeque;

use futures::{
    future::FutureExt,
    task::{Context, Poll, Waker},
};
use libp2p::{
    core::upgrade::{DeniedUpgrade, InboundUpgrade, OutboundUpgrade},
    swarm::{
        ConnectionHandler, ConnectionHandlerEvent, ConnectionHandlerUpgrErr, KeepAlive,
        NegotiatedSubstream, SubstreamProtocol,
    },
    PeerId,
};
use thiserror::Error;
use tokio::sync::oneshot;

use beserial::SerializingError;
use nimiq_network_interface::peer::CloseReason;

#[derive(Clone, Debug)]
pub enum HandlerInEvent {
    Close { reason: CloseReason },
    PeerConnected { peer_id: PeerId, outbound: bool },
}

#[derive(Debug)]
pub enum HandlerOutEvent {
    PeerJoined {
        close_tx: oneshot::Sender<CloseReason>,
    },
    ClosePeerConnection {
        reason: CloseReason,
    },
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Serialization error: {0}")]
    Serializing(#[from] SerializingError),

    #[error("Connection closed: reason={:?}", {0})]
    ConnectionClosed { reason: CloseReason },
}

// TODO: Refactor state into enum
pub struct ConnectionPoolHandler {
    peer_id: Option<PeerId>,

    pending_peer_announcement: bool,

    // Receives the close reason when `close()` is called on the peer.
    close_rx: Option<oneshot::Receiver<CloseReason>>,

    waker: Option<Waker>,

    events: VecDeque<ConnectionHandlerEvent<DeniedUpgrade, (), HandlerOutEvent, HandlerError>>,

    /// The sub-stream while we're polling it for closing.
    closing: Option<CloseReason>,
}

impl ConnectionPoolHandler {
    pub fn new() -> Self {
        Self {
            peer_id: None,
            pending_peer_announcement: false,
            close_rx: None,
            waker: None,
            events: VecDeque::new(),
            closing: None,
        }
    }

    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}

// Implement ConnectionHandler without an actual protocol, which
// implies a DeniedUpgrade protocol
impl ConnectionHandler for ConnectionPoolHandler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = HandlerError;
    type InboundProtocol = DeniedUpgrade;
    type OutboundProtocol = DeniedUpgrade;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<DeniedUpgrade, ()> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }

    fn inject_fully_negotiated_inbound(
        &mut self,
        _: <DeniedUpgrade as InboundUpgrade<NegotiatedSubstream>>::Output,
        _info: (),
    ) {
        unreachable!("`DeniedUpgrade` is never successful.")
    }

    fn inject_fully_negotiated_outbound(
        &mut self,
        _: <DeniedUpgrade as OutboundUpgrade<NegotiatedSubstream>>::Output,
        _info: (),
    ) {
        unreachable!("`DeniedUpgrade` is never successful.")
    }

    fn inject_event(&mut self, event: HandlerInEvent) {
        log::trace!("inject_event: {:?}", event);

        match event {
            HandlerInEvent::Close { reason } => {
                if let Some(peer_id) = &self.peer_id {
                    if self.closing.is_some() {
                        log::trace!("Socket closing pending");
                    } else {
                        log::debug!("ConnectionPoolHandler: Closing peer: {}", peer_id);
                        self.closing = Some(reason);
                        self.close_rx = None;
                    }
                }
            }
            HandlerInEvent::PeerConnected {
                peer_id,
                outbound: _,
            } => {
                // peer_id should not have been set yet.
                assert!(self.peer_id.is_none());

                self.peer_id = Some(peer_id);
                self.pending_peer_announcement = true;

                self.wake();
            }
        }
    }

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        _error: ConnectionHandlerUpgrErr<
            <DeniedUpgrade as OutboundUpgrade<NegotiatedSubstream>>::Error,
        >,
    ) {
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }

    fn poll(
        &mut self,
        cx: &mut Context,
    ) -> Poll<ConnectionHandlerEvent<DeniedUpgrade, (), HandlerOutEvent, HandlerError>> {
        #[allow(clippy::never_loop)]
        loop {
            // Emit event
            if let Some(event) = self.events.pop_front() {
                return Poll::Ready(event);
            }

            if let Some(peer_id) = self.peer_id {
                // Poll the oneshot receiver that signals us when the peer is closed
                if let Some(close_rx) = &mut self.close_rx {
                    match close_rx.poll_unpin(cx) {
                        Poll::Ready(Ok(reason)) => {
                            log::debug!("ConnectionPoolHandler: Closing peer: {}", peer_id);
                            self.closing = Some(reason);
                        }
                        Poll::Ready(Err(e)) => panic!("close_rx returned error: {}", e), // Channel was closed without message.
                        Poll::Pending => {}
                    }
                }

                // If we're currently closing the socket, call poll_close on it, until it finishes.
                if let Some(reason) = self.closing {
                    log::trace!("Polling socket to close: reason={:?}", reason);

                    self.closing = None;
                    self.peer_id = None;

                    // Gracefully close the connection
                    return Poll::Ready(ConnectionHandlerEvent::Custom(
                        HandlerOutEvent::ClosePeerConnection { reason },
                    ));
                }
            }

            // Wait for outbound and inbound to be established and the peer ID to be injected.
            if self.peer_id.is_none() || !self.pending_peer_announcement {
                break;
            }

            assert!(self.close_rx.is_none());

            // Take inbound and outbound and create a peer from it.
            let peer_id = self.peer_id.unwrap();
            self.pending_peer_announcement = false;

            // Create a channel that is used to receive the close signal from the `Peer` struct (when `Peer::close` is called).
            let (close_tx, close_rx) = oneshot::channel();

            log::debug!("New peer: {}", peer_id);

            self.close_rx = Some(close_rx);

            // Send peer to behaviour
            return Poll::Ready(ConnectionHandlerEvent::Custom(
                HandlerOutEvent::PeerJoined { close_tx },
            ));
        }

        store_waker!(self, waker, cx);

        Poll::Pending
    }
}
