use libp2p::swarm::{ProtocolsHandler, SubstreamProtocol, ProtocolsHandlerUpgrErr, KeepAlive, ProtocolsHandlerEvent};
use futures::{
    task::{Context, Poll},
};
use thiserror::Error;


use super::protocol::LimitProtocol;


#[derive(Clone, Debug)]
pub enum HandlerInEvent {
}


#[derive(Clone, Debug)]
pub enum HandlerOutEvent {
}


#[derive(Debug, Error)]
pub enum HandlerError {
}

#[derive(Default)]
pub struct LimitHandler {
}

impl ProtocolsHandler for LimitHandler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = HandlerError;
    type InboundProtocol = LimitProtocol;
    type OutboundProtocol = LimitProtocol;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<LimitProtocol, ()> {
        log::debug!("LimitHandler::listen_protocol");
        SubstreamProtocol::new(LimitProtocol, ())
    }

    fn inject_fully_negotiated_inbound(&mut self, _protocol: (), _info: ()) {
        log::debug!("LimitHandler::inject_fully_negotiated_inbound");
        todo!();
    }

    fn inject_fully_negotiated_outbound(&mut self, _protocol: (), _info: ()) {
        todo!();
    }

    fn inject_event(&mut self, event: HandlerInEvent) {
        todo!();
    }

    fn inject_dial_upgrade_error(&mut self, _info: Self::OutboundOpenInfo, error: ProtocolsHandlerUpgrErr<std::io::Error>) {
        log::warn!("DiscoveryHandler::inject_dial_upgrade_error: {:?}", error);
        unimplemented!();
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::No
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<ProtocolsHandlerEvent<Self::OutboundProtocol, (), HandlerOutEvent, HandlerError>> {
        // We do nothing in the handler
        Poll::Pending
    }
}
