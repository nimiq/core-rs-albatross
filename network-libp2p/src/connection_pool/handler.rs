use libp2p::swarm::{
    protocols_handler::{InboundUpgradeSend, OutboundUpgradeSend},
    KeepAlive, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol,
};
use std::{
    fmt::Debug,
    task::{Context, Poll},
};
use thiserror::Error;

use super::protocol::ConnectionPoolProtocol;

#[derive(Clone, Debug)]
pub enum HandlerInEvent {}

#[derive(Clone, Debug)]
pub enum HandlerOutEvent {}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("PeersBehaviour error")]
    PeersError,
}

pub struct ConnectionPoolHandler {}

impl ConnectionPoolHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl ProtocolsHandler for ConnectionPoolHandler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = HandlerError;
    type InboundProtocol = ConnectionPoolProtocol;
    type OutboundProtocol = ConnectionPoolProtocol;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<ConnectionPoolProtocol, ()> {
        SubstreamProtocol::new(ConnectionPoolProtocol, ())
    }

    fn inject_fully_negotiated_inbound(
        &mut self,
        _protocol: <Self::InboundProtocol as InboundUpgradeSend>::Output,
        _info: Self::InboundOpenInfo,
    ) {
    }

    fn inject_fully_negotiated_outbound(
        &mut self,
        _protocol: <Self::OutboundProtocol as OutboundUpgradeSend>::Output,
        _info: Self::OutboundOpenInfo,
    ) {
    }

    fn inject_event(&mut self, _event: Self::InEvent) {}

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        _error: ProtocolsHandlerUpgrErr<<Self::OutboundProtocol as OutboundUpgradeSend>::Error>,
    ) {
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::No
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<
        ProtocolsHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::OutEvent,
            Self::Error,
        >,
    > {
        Poll::Pending
    }
}
