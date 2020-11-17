use std::{
    collections::VecDeque,
    sync::Arc,
    io,
};

use futures::{
    io::ErrorKind,
    task::{Context, Poll, Waker},
    channel::mpsc,
};
use libp2p::{
    swarm::{
        KeepAlive, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol,
        NegotiatedSubstream,
    },
    PeerId,
};
use thiserror::Error;

use beserial::SerializingError;

use super::{
    peer::{Peer, PeerAction},
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
    PeerDisconnect {
        peer_id: PeerId,
    }
}

#[derive(Clone, Debug)]
pub enum HandlerOutEvent {
    PeerJoined {
        peer: Arc<Peer>,
    }
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Serialization error: {0}")]
    Serializing(#[from] SerializingError),
}


pub struct MessageHandler {
    config: MessageConfig,

    peer_id: Option<PeerId>,

    peer: Option<Arc<Peer>>,

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

            HandlerInEvent::PeerDisconnect { peer_id } => {
                assert!(self.peer.is_some());

                todo!("FIXME: Peer disconnected");

                self.peer = None;
                self.wake(); // necessary?
            }
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

            // Take inbound and outbound and create a peer from it.
            let peer_id = self.peer_id.clone().unwrap();
            let socket = self.socket.take().unwrap();

            let peer = Arc::new(Peer::new(peer_id, socket));
            log::debug!("New peer: {:?}", peer);

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
