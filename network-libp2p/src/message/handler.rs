use std::{collections::VecDeque, sync::Arc};

use futures::{
    channel::oneshot,
    future::BoxFuture,
    task::{Context, Poll, Waker},
    FutureExt,
};
use libp2p::{
    swarm::{KeepAlive, NegotiatedSubstream, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol},
    PeerId,
};
use nimiq_network_interface::peer::CloseReason;
use thiserror::Error;

use beserial::SerializingError;

use super::{behaviour::MessageConfig, dispatch::MessageDispatch, peer::Peer, protocol::MessageProtocol};

#[derive(Clone, Debug)]
pub enum HandlerInEvent {
    PeerConnected { peer_id: PeerId, outbound: bool },
}

#[derive(Clone, Debug)]
pub enum HandlerOutEvent {
    PeerJoined { peer: Arc<Peer> },
    PeerClosed { peer: Arc<Peer>, reason: CloseReason },
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Serialization error: {0}")]
    Serializing(#[from] SerializingError),

    #[error("Connection closed: reason={:?}", {0})]
    ConnectionClosed { reason: CloseReason },
}

pub struct MessageHandler {
    // NOTE: Will probably be used later.
    #[allow(dead_code)]
    config: MessageConfig,

    peer_id: Option<PeerId>,

    peer: Option<Arc<Peer>>,

    close_rx: Option<oneshot::Receiver<CloseReason>>,

    waker: Option<Waker>,

    events: VecDeque<ProtocolsHandlerEvent<MessageProtocol, (), HandlerOutEvent, HandlerError>>,

    socket: Option<MessageDispatch<NegotiatedSubstream>>,

    closing: Option<(BoxFuture<'static, ()>, CloseReason)>,
}

impl MessageHandler {
    pub fn new(config: MessageConfig) -> Self {
        Self {
            config,
            peer_id: None,
            peer: None,
            close_rx: None,
            waker: None,
            events: VecDeque::new(),
            socket: None,
            closing: None,
        }
    }

    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}

impl ProtocolsHandler for MessageHandler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = HandlerError;
    type InboundProtocol = MessageProtocol;
    type OutboundProtocol = MessageProtocol;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<MessageProtocol, ()> {
        SubstreamProtocol::new(MessageProtocol::default(), ())
    }

    fn inject_fully_negotiated_inbound(&mut self, socket: MessageDispatch<NegotiatedSubstream>, _info: ()) {
        log::debug!("MessageHandler::inject_fully_negotiated_inbound");

        if self.peer.is_none() && self.socket.is_none() {
            self.socket = Some(socket);
            self.wake();
        } else {
            log::debug!("Connection already established. Ignoring inbound.");
        }
    }

    fn inject_fully_negotiated_outbound(&mut self, socket: MessageDispatch<NegotiatedSubstream>, _info: ()) {
        log::debug!("MessageHandler::inject_fully_negotiated_outbound");

        if self.peer.is_none() && self.socket.is_none() {
            self.socket = Some(socket);
            self.wake();
        } else {
            log::debug!("Connection already established. Ignoring outbound.");
        }
    }

    fn inject_event(&mut self, event: HandlerInEvent) {
        log::debug!("MessageHandler::inject_event: {:?}", event);

        match event {
            HandlerInEvent::PeerConnected { peer_id, outbound } => {
                assert!(self.peer_id.is_none());

                log::debug!("Requesting outbound substream.");

                self.peer_id = Some(peer_id);

                if outbound {
                    // Next open the outbound
                    self.events.push_back(ProtocolsHandlerEvent::OutboundSubstreamRequest {
                        protocol: SubstreamProtocol::new(MessageProtocol::default(), ()),
                    });
                    self.wake();
                }
            }
            /*HandlerInEvent::PeerDisconnected => {
                unreachable!();
                // FIXME: Actually I think this is never called.
                // If `self.peer` is `None`, it was closed by this handler already.
                // TODO: We can expect `self.peer` to be Some here.
                if let Some(peer) = self.peer.take() {
                    self.socket = None;
                    self.close_rx = None;
                    self.keep_alive = KeepAlive::No;

                    log::debug!("Peer disconnected: {:?}", peer);

                    self.events.push_back(ProtocolsHandlerEvent::Custom(HandlerOutEvent::PeerClosed {
                        reason: CloseReason::Other, // TODO: We might have a reason from the close_rx
                        peer
                    }));

                    self.wake();
                }
            },*/
        }
    }

    fn inject_dial_upgrade_error(&mut self, _info: Self::OutboundOpenInfo, error: ProtocolsHandlerUpgrErr<SerializingError>) {
        // TODO handle this
        panic!("Dial upgrade error: {}", error);
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<ProtocolsHandlerEvent<MessageProtocol, (), HandlerOutEvent, HandlerError>> {
        loop {
            log::trace!("MessageHandler::poll - Iteration");

            // Emit event
            if let Some(event) = self.events.pop_front() {
                log::trace!("MessageHandler: emitting event: {:?}", event);
                return Poll::Ready(event);
            }

            // Check if we're closing this connection
            if let Some((close_fut, reason)) = &mut self.closing {
                log::trace!("Polling closing future");
                match close_fut.poll_unpin(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(_) => {
                        // Close the handler
                        log::trace!("Closing MessageHandler.");

                        return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::ConnectionClosed { reason: *reason }));
                    }
                }
            }

            // Poll the oneshot receiver that signals us when the peer is closed
            if let Some(close_rx) = &mut self.close_rx {
                match close_rx.poll_unpin(cx) {
                    Poll::Ready(Ok(reason)) => {
                        let peer = self.peer.take().expect("Expected peer");

                        log::debug!("MessageHandler: Closing peer: {:?}", peer);

                        self.closing = Some((async move { peer.socket.close().await }.boxed(), reason));

                        continue;
                    }
                    Poll::Ready(Err(e)) => panic!("close_rx returned error: {}", e), // Channel was closed without message.
                    Poll::Pending => {}
                }
            }

            if let Some(peer) = &self.peer {
                // Poll the MessageReceiver if an error occured
                match peer.socket.inbound.poll_error(cx) {
                    Poll::Ready(Some(e)) => {
                        // TODO: Check if `e` is actually an EOF and not just another error. And put that error into
                        //       the `CloseReason`.
                        log::warn!("Remote closed connection: {}", e);

                        return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::ConnectionClosed {
                            reason: CloseReason::RemoteClosed,
                        }));
                    }
                    _ => {}
                }
            }

            // If the peer is already available, poll it and return
            // TODO: Poll the future in the MessageReceiver
            /*if let Some(peer) = self.peer.as_mut() {
                match peer.poll(cx) {
                    Poll::Ready(Ok(())) => unreachable!(),
                    Poll::Ready(Err(e)) => {
                        log::error!("Peer future failed: {}", e);
                        return Poll::Ready(ProtocolsHandlerEvent::Close(e.into()))
                    },
                    Poll::Pending => return Poll::Pending,
                }
            }*/

            // Wait for outbound and inbound to be established and the peer ID to be injected.
            if self.socket.is_none() || self.peer_id.is_none() {
                break;
            }

            assert!(self.peer.is_none());
            assert!(self.close_rx.is_none());

            // Take inbound and outbound and create a peer from it.
            let peer_id = self.peer_id.clone().unwrap();
            let socket = self.socket.take().unwrap();

            let (close_tx, close_rx) = oneshot::channel();

            let peer = Arc::new(Peer::new(peer_id, socket, close_tx));
            log::debug!("New peer: {:?}", peer);

            self.close_rx = Some(close_rx);
            self.peer = Some(Arc::clone(&peer));

            // Send peer to behaviour
            return Poll::Ready(ProtocolsHandlerEvent::Custom(HandlerOutEvent::PeerJoined { peer }));
        }

        // Remember the waker
        if self.waker.is_none() {
            self.waker = Some(cx.waker().clone());
        }

        Poll::Pending
    }
}
