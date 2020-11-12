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
    dispatch::{MessageSender, MessageReceiver},
    behaviour::MessageConfig,
};

#[derive(Clone, Debug)]
pub enum HandlerInEvent {
    PeerJoined {
        peer_id: PeerId,
    },
    PeerLeft {
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

    inbound: Option<MessageReceiver>,

    outbound: Option<MessageSender<NegotiatedSubstream>>,
}

impl MessageHandler {
    pub fn new(config: MessageConfig) -> Self {
        Self {
            config,
            peer_id: None,
            peer: None,
            waker: None,
            events: VecDeque::new(),
            inbound: None,
            outbound: None,
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

    fn inject_fully_negotiated_inbound(&mut self, inbound: MessageReceiver, _info: ()) {
        log::debug!("MessageHandler::inject_fully_negotiated_inbound");
        assert!(self.inbound.is_none());
        self.inbound = Some(inbound);
        self.wake();
    }

    fn inject_fully_negotiated_outbound(&mut self, outbound: MessageSender<NegotiatedSubstream>, _info: ()) {
        log::debug!("MessageHandler::inject_fully_negotiated_outbound");
        assert!(self.outbound.is_none());
        self.outbound = Some(outbound);
        self.wake();
    }

    fn inject_event(&mut self, event: HandlerInEvent) {
        log::debug!("MessageHandler::inject_event: {:?}", event);

        match event {
            HandlerInEvent::PeerJoined { peer_id } => {
                assert!(self.peer_id.is_none());

                self.peer_id = Some(peer_id);

                // Next open the outbound
                self.events.push_back(ProtocolsHandlerEvent::OutboundSubstreamRequest {
                    protocol: SubstreamProtocol::new(MessageProtocol::default(), ())
                });
                self.wake();
            },

            HandlerInEvent::PeerLeft { peer_id } => {
                assert!(self.peer.is_some());
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
        error!("Dial upgrade error: {:?}", error);
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<ProtocolsHandlerEvent<MessageProtocol, (), HandlerOutEvent, HandlerError>> {
        loop {
            // Emit event
            if let Some(event) = self.events.pop_front() {
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
            if self.inbound.is_none() || self.outbound.is_none() || self.peer_id.is_none() {
                break;
            }

            // Take inbound and outbound and create a peer from it.
            let peer_id = self.peer_id.clone().unwrap();
            let inbound = self.inbound.take().unwrap();
            let outbound = self.outbound.take().unwrap();

            let peer = Arc::new(Peer::new(peer_id, inbound, outbound));

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
