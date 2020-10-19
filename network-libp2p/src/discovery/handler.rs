use std::pin::Pin;

use libp2p::swarm::{
    ProtocolsHandler, SubstreamProtocol, ProtocolsHandlerUpgrErr, KeepAlive, ProtocolsHandlerEvent, NegotiatedSubstream,
};
use futures::{
    task::{Context, Poll},
    Stream, StreamExt, Sink, SinkExt, ready,
};

use beserial::SerializingError;

use super::protocol::{DiscoveryProtocol, DiscoveryMessage};
use crate::message::{MessageReader, MessageWriter};


pub struct DiscoveryHandler {
    inbound: Option<MessageReader<NegotiatedSubstream, DiscoveryMessage>>,
    outbound: Option<MessageWriter<NegotiatedSubstream, DiscoveryMessage>>,
}

impl ProtocolsHandler for DiscoveryHandler {
    type InEvent = ();
    type OutEvent = DiscoveryMessage;
    type Error = SerializingError;
    type InboundProtocol = DiscoveryProtocol;
    type OutboundProtocol = DiscoveryProtocol;
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol> {
        SubstreamProtocol::new(DiscoveryProtocol)
    }

    fn inject_fully_negotiated_inbound(&mut self, protocol: MessageReader<NegotiatedSubstream, DiscoveryMessage>) {
        self.inbound = Some(protocol);
    }

    fn inject_fully_negotiated_outbound(&mut self, protocol: MessageWriter<NegotiatedSubstream, DiscoveryMessage>, info: ()) {
        self.outbound = Some(protocol);
    }

    fn inject_event(&mut self, event: Self::InEvent) {
    }

    fn inject_dial_upgrade_error(&mut self, info: Self::OutboundOpenInfo, error: ProtocolsHandlerUpgrErr<SerializingError>) {
        unimplemented!()
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::No
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<ProtocolsHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::OutEvent, Self::Error>> {
        // Receive message
        if let Some(inbound) = self.inbound.as_mut() {
            match ready!(inbound.poll_next_unpin(cx)) {
                Some(Ok(message)) => return Poll::Ready(ProtocolsHandlerEvent::Custom(message)),
                None => return Poll::Ready(ProtocolsHandlerEvent::Close(std::io::Error::from(std::io::ErrorKind::ConnectionReset).into())),
                Some(Err(e)) => return Poll::Ready(ProtocolsHandlerEvent::Close(e))
            }
        }

        // Send message
        if let Some(outbound) = self.outbound.as_mut() {
            match ready!(outbound.poll_ready_unpin(cx)) {
                Err(e) => return Poll::Ready(ProtocolsHandlerEvent::Close(e)),
                _ => {},
            }
        }

        Poll::Pending
    }
}
