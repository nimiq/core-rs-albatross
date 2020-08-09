use crate::protocol::NimiqProtocol;
use futures::io::ErrorKind;
use futures::task::{Context, Poll};
use libp2p::swarm::{
    KeepAlive, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol,
};
use std::collections::VecDeque;
use std::io;

#[derive(Clone, Debug)]
pub enum NimiqHandlerAction {
    Message(Vec<u8>),
    Close,
}

pub struct NimiqHandler {
    inbound_msgs: VecDeque<Vec<u8>>,
    outbound_msgs: VecDeque<Vec<u8>>,
    closed: bool,
}

impl NimiqHandler {
    pub(crate) fn new() -> Self {
        Self {
            inbound_msgs: VecDeque::new(),
            outbound_msgs: VecDeque::new(),
            closed: false,
        }
    }
}

impl ProtocolsHandler for NimiqHandler {
    type InEvent = NimiqHandlerAction;
    type OutEvent = Vec<u8>;
    type Error = io::Error;
    type InboundProtocol = NimiqProtocol;
    type OutboundProtocol = NimiqProtocol;
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol> {
        SubstreamProtocol::new(NimiqProtocol::inbound())
    }

    fn inject_fully_negotiated_inbound(&mut self, msg: Vec<u8>) {
        self.inbound_msgs.push_back(msg);
    }

    fn inject_fully_negotiated_outbound(&mut self, _: (), _info: Self::OutboundOpenInfo) {}

    fn inject_event(&mut self, action: NimiqHandlerAction) {
        match action {
            NimiqHandlerAction::Message(msg) => self.outbound_msgs.push_back(msg),
            NimiqHandlerAction::Close => self.closed = true,
        }
    }

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        error: ProtocolsHandlerUpgrErr<Self::Error>,
    ) {
        // TODO handle this
        error!("Dial upgrade error: {:?}", error);
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }

    fn poll(
        &mut self,
        _cx: &mut Context,
    ) -> Poll<
        ProtocolsHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::OutEvent,
            Self::Error,
        >,
    > {
        if self.closed {
            // FIXME Close error
            // TODO return None after closing?
            return Poll::Ready(ProtocolsHandlerEvent::Close(io::Error::from(
                ErrorKind::ConnectionAborted,
            )));
        }

        if let Some(msg) = self.inbound_msgs.pop_front() {
            return Poll::Ready(ProtocolsHandlerEvent::Custom(msg));
        }

        if let Some(msg) = self.outbound_msgs.pop_front() {
            return Poll::Ready(ProtocolsHandlerEvent::OutboundSubstreamRequest {
                protocol: SubstreamProtocol::new(NimiqProtocol::outbound(msg)),
                info: (),
            });
        }

        Poll::Pending
    }
}
