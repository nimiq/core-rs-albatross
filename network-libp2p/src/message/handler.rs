use std::{
    collections::VecDeque,
    sync::Arc,
    pin::Pin,
};

use futures::{
    task::{Context, Poll, Waker},
    channel::oneshot,
    Future,
};
use libp2p::{
    swarm::{
        KeepAlive, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol,
        NegotiatedSubstream,
    },
    PeerId,
};
use thiserror::Error;
use nimiq_network_interface::peer::CloseReason;

use beserial::SerializingError;


use super::{
    peer::Peer,
    protocol::MessageProtocol,
    dispatch::MessageDispatch,
    behaviour::MessageConfig,
};

#[derive(Clone, Debug)]
pub enum HandlerInEvent {
    PeerConnected {
        peer_id: PeerId,
        outbound: bool,
    },
    //PeerDisconnected,
}

#[derive(Clone, Debug)]
pub enum HandlerOutEvent {
    PeerJoined {
        peer: Arc<Peer>,
    },
    PeerClosed {
        peer: Arc<Peer>,
        reason: CloseReason,
    },
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Serialization error: {0}")]
    Serializing(#[from] SerializingError),

    #[error("Connection closed: reason={:?}", {0})]
    ConnectionClosed {
        reason: CloseReason,
    },
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
        }
        else {
            log::debug!("Connection already established. Ignoring inbound.");
        }
    }

    fn inject_fully_negotiated_outbound(&mut self, socket: MessageDispatch<NegotiatedSubstream>, _info: ()) {
        log::debug!("MessageHandler::inject_fully_negotiated_outbound");

        if self.peer.is_none() && self.socket.is_none() {
            self.socket = Some(socket);
            self.wake();
        }
        else {
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
                        protocol: SubstreamProtocol::new(MessageProtocol::default(), ())
                    });
                    self.wake();
                }
            },

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

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        error: ProtocolsHandlerUpgrErr<SerializingError>,
    ) {
        // TODO handle this
        panic!("Dial upgrade error: {}", error);
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<ProtocolsHandlerEvent<MessageProtocol, (), HandlerOutEvent, HandlerError>> {
        loop {
            // Emit event
            if let Some(event) = self.events.pop_front() {
                log::debug!("MessageHandler: emitting event: {:?}", event);
                return Poll::Ready(event);
            }

            // Poll the oneshot receiver that signals us when the peer is closed
            if let Some(close_rx) = &mut self.close_rx {
                match Pin::new(close_rx).poll(cx) { // TODO: use .poll_unpin()
                    Poll::Ready(Ok(reason)) => {
                        let peer = Arc::clone(self.peer.as_ref().expect("Expected peer"));

                        log::debug!("MessageHandler: Closing peer: {:?}", peer);

                        // Drop peer and socket
                        // This is not needed as the returned event will cause the handler to be dropped.
                        /*self.peer = None;
                        self.socket = None;
                        self.close_rx = None;

                        // Close connection as soon as possible
                        self.keep_alive = KeepAlive::No;*/

                        // If the peer signals use that they were closed, we emit that event to the behaviour.
                        /*return Poll::Ready(ProtocolsHandlerEvent::Custom(HandlerOutEvent::PeerClosed {
                            reason,
                            peer,
                        }));*/

                        return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::ConnectionClosed {
                            reason,
                        }));
                    },
                    Poll::Ready(Err(_)) => unimplemented!(), // Channel was closed without message.
                    Poll::Pending => {},
                }
            }

            if let Some(peer) = &self.peer {
                // Poll the MessageReceiver if an error occured
                match peer.socket.inbound.poll_error(cx) {
                    Poll::Ready(Some(e)) => {
                        // TODO: Check if `e` is actually an EOF and not just another error. And put that error into
                        //       the `CloseReason`.
                        log::debug!("Remote closed connection: {}", e);

                        return Poll::Ready(ProtocolsHandlerEvent::Close(HandlerError::ConnectionClosed {
                            reason: CloseReason::RemoteClosed
                        }))
                    },
                    _ => {},
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
