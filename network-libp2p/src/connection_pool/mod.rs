pub mod behaviour;
pub use behaviour::{Behaviour, Event};
use thiserror::Error;

/// Connection Pool errors
#[derive(Clone, Debug, Error)]
pub enum Error {
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
}
