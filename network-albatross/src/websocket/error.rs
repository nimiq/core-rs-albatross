use native_tls::Error as TlsError;
use thiserror::Error;
use tokio::io::Error as IoError;
use tokio::timer::Error as TimerError;
use tungstenite::error::Error as WsError;
use url::ParseError;

use beserial::SerializingError;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    WebSocketError(#[from] WsError),
    #[error("Received a message with an unexpected tag")]
    TagMismatch,
    #[error("{0}")]
    ParseError(#[from] SerializingError),
    #[error("Received a chunk with a size exceeding the defined maximum")]
    ChunkSizeExceeded,
    #[error("Received a message with a size exceeding the defined maximum")]
    MessageSizeExceeded,
    #[error("Received the final chunk with a size exceeding the expected size")]
    FinalChunkSizeExceeded,
    #[error("Tried closing a connection and got invalid response from the WebSocket layer")]
    InvalidClosingState,
    #[error("Stream could not be wrapped: TLS acceptor is None")]
    TlsAcceptorMissing,
    #[error("Stream could not be wrapped: {0}")]
    TlsWrappingError(#[from] TlsError),
    #[error("{0}")]
    IoError(#[from] IoError),
    #[error("Could not read net address from stream: {0}")]
    NetAddressMissing(IoError),
    #[error("Message format is incorrect and could not be parsed correctly")]
    InvalidMessageFormat,
}

// This implementation is needed for forwarding into our Sink.
impl From<Error> for () {
    fn from(_: Error) -> Self {}
}

#[derive(Error, Debug)]
pub enum ConnectError {
    #[error("Connection timed out")]
    Timeout,
    #[error("Protocol flags do not match")]
    ProtocolMismatch,
    #[error("Connection was aborted by us")]
    AbortedByUs,
    #[error("{0}")]
    Timer(#[from] TimerError),
    #[error("{0}")]
    WebSocket(#[from] Error),
    #[error("Could not parse URI to connect to: {0}")]
    InvalidUri(#[from] ParseError),
}

#[derive(Error, Debug)]
pub enum ServerStartError {
    #[error("TLS certificate is missing or could not be read")]
    CertificateMissing,
    #[error("Wrong TLS certificate passphrase")]
    CertificatePassphraseError,
    #[error("{0}")]
    IoError(#[from] IoError),
    #[error("{0}")]
    TlsError(#[from] TlsError),
    #[error("Protocol config is not supported: {0}")]
    UnsupportedProtocol(String),
}
