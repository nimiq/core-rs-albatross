use nimiq_network_interface::{network::SendError, request::RequestError};
use thiserror::Error;

/// No notion of connected or disconnected!
/// If a peer is not connected the connection must be pursued.
/// If establishing the connection is impossible (i.e. Peer is offline or Self is offline) Unreachable is used.
#[derive(Debug, Error)]
pub enum NetworkError<TNetworkError>
where
    TNetworkError: std::error::Error + 'static,
{
    /// Some of the peers were unreachable
    #[error("Unreachable")]
    Unreachable,

    #[error("We are not an elected validator")]
    NotElected,

    /// If no specific set of peers was given but no connection could be established indicating that self is unreachable
    #[error("Network is offline")]
    Offline,

    /// The public key for that validator is not known.
    #[error("Unknown validator: {0}")]
    UnknownValidator(u16),

    #[error("Network error: {0}")]
    Network(#[from] TNetworkError),

    #[error("Send error: {0}")]
    Send(SendError),

    #[error("Request error: {0}")]
    Request(RequestError),
}
