use libp2p::{
    core::upgrade::{DeniedUpgrade, InboundUpgrade, OutboundUpgrade},
    swarm::{
        ConnectionHandler, ConnectionHandlerEvent, ConnectionHandlerUpgrErr, KeepAlive,
        NegotiatedSubstream, SubstreamProtocol,
    },
};
use std::task::{Context, Poll};
use thiserror::Error;

#[derive(Default)]
pub struct ConnectionPoolHandler {}

#[derive(Debug, Error)]
pub enum ConnectionPoolHandlerError {}

// Implement ConnectionHandler without an actual protocol, which
// implies a DeniedUpgrade protocol
impl ConnectionHandler for ConnectionPoolHandler {
    type InEvent = ();
    type OutEvent = ();
    type Error = ConnectionPoolHandlerError;
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

    fn inject_event(&mut self, _event: ()) {}

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
        _cx: &mut Context,
    ) -> Poll<ConnectionHandlerEvent<DeniedUpgrade, (), (), ConnectionPoolHandlerError>> {
        Poll::Pending
    }
}
