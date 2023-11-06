use std::task::{Context, Poll};

use libp2p::{
    core::upgrade::{DeniedUpgrade, InboundUpgrade, OutboundUpgrade},
    swarm::{
        ConnectionHandler, ConnectionHandlerEvent, ConnectionHandlerUpgrErr, KeepAlive,
        NegotiatedSubstream, SubstreamProtocol,
    },
};
use nimiq_network_interface::network::CloseReason;
use thiserror::Error;

#[derive(Default)]
pub struct ConnectionPoolHandler {
    /// If set to `Some` it indicates that the current connection handler
    /// must be closed with the reason specified.
    close_reason: Option<ConnectionPoolHandlerError>,
}

/// Connection Pool errors
#[derive(Clone, Debug, Error)]
pub enum ConnectionPoolHandlerError {
    /// There is already a connection for this Peer
    #[error("Peer is already connected")]
    AlreadyConnected,

    /// Ip is banned
    #[error("IP is banned")]
    BannedIp,

    /// Peer is banned
    #[error("Peer is banned")]
    BannedPeer,

    /// Maximum connections per subnet has been reached
    #[error("Maximum connections per subnet has been reached")]
    MaxSubnetConnectionsReached,

    /// Maximum peer connections has been reached
    #[error("Maximum peer connections has been reached")]
    MaxPeerConnectionsReached,

    ///Maximum peers connections per IP has been reached
    #[error("Maximum peers connections per IP has been reached")]
    MaxPeerPerIPConnectionsReached,

    /// The application sent the network to close the connection with the
    /// provided reason
    #[error("Application sent a close action with reason: {0:?}")]
    Other(CloseReason),
}

// Implement ConnectionHandler without an actual protocol, which
// implies a DeniedUpgrade protocol
impl ConnectionHandler for ConnectionPoolHandler {
    type InEvent = ConnectionPoolHandlerError; // Only receive errors as events for this handler
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

    fn inject_event(&mut self, close_reason: ConnectionPoolHandlerError) {
        if self.close_reason.is_none() {
            self.close_reason = Some(close_reason);
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
        _cx: &mut Context,
    ) -> Poll<ConnectionHandlerEvent<DeniedUpgrade, (), (), ConnectionPoolHandlerError>> {
        if let Some(reason) = self.close_reason.take() {
            return Poll::Ready(ConnectionHandlerEvent::Close(reason));
        }
        Poll::Pending
    }
}
