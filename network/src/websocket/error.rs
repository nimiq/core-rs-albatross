use failure::Fail;
use native_tls::Error as TlsError;
use tokio::io::Error as IoError;
use tokio::time::Error as TimerError;
use tungstenite::error::Error as WsError;
use url::ParseError;

use beserial::SerializingError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    WebSocketError(#[cause] WsError),
    #[fail(display = "Received a message with an unexpected tag")]
    TagMismatch,
    #[fail(display = "{}", _0)]
    ParseError(#[cause] SerializingError),
    #[fail(display = "Received a chunk with a size exceeding the defined maximum")]
    ChunkSizeExceeded,
    #[fail(display = "Received a message with a size exceeding the defined maximum")]
    MessageSizeExceeded,
    #[fail(display = "Received the final chunk with a size exceeding the expected size")]
    FinalChunkSizeExceeded,
    #[fail(display = "Tried closing a connection and got invalid response from the WebSocket layer")]
    InvalidClosingState,
    #[fail(display = "Stream could not be wrapped: TLS acceptor is None")]
    TlsAcceptorMissing,
    #[fail(display = "Stream could not be wrapped: {}", _0)]
    TlsWrappingError(#[cause] TlsError),
    #[fail(display = "{}", _0)]
    IoError(#[cause] IoError),
    #[fail(display = "Could not read net address from stream: {}", _0)]
    NetAddressMissing(#[cause] IoError),
    #[fail(display = "Message format is incorrect and could not be parsed correctly")]
    InvalidMessageFormat,
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Self {
        Error::IoError(e)
    }
}

impl From<WsError> for Error {
    fn from(e: WsError) -> Self {
        Error::WebSocketError(e)
    }
}

impl From<SerializingError> for Error {
    fn from(e: SerializingError) -> Self {
        Error::ParseError(e)
    }
}

// This implementation is needed for forwarding into our Sink.
impl From<Error> for () {
    fn from(_: Error) -> Self {

    }
}

#[derive(Fail, Debug)]
pub enum ConnectError {
    #[fail(display = "Connection timed out")]
    Timeout,
    #[fail(display = "Protocol flags do not match")]
    ProtocolMismatch,
    #[fail(display = "Connection was aborted by us")]
    AbortedByUs,
    #[fail(display = "{}", _0)]
    Timer(#[cause] TimerError),
    #[fail(display = "{}", _0)]
    WebSocket(#[cause] Error),
    #[fail(display = "Could not parse URI to connect to: {}", _0)]
    InvalidUri(#[cause] ParseError),
}


impl From<TimerError> for ConnectError {
    fn from(e: TimerError) -> Self {
        ConnectError::Timer(e)
    }
}


impl From<Error> for ConnectError {
    fn from(e: Error) -> Self {
        ConnectError::WebSocket(e)
    }
}

#[derive(Fail, Debug)]
pub enum ServerStartError {
    #[fail(display = "TLS certificate is missing or could not be read")]
    CertificateMissing,
    #[fail(display = "Wrong TLS certificate passphrase")]
    CertificatePassphraseError,
    #[fail(display = "{}", _0)]
    IoError(#[cause] IoError),
    #[fail(display = "{}", _0)]
    TlsError(#[cause] TlsError),
    #[fail(display = "Protocol config is not supported: {}", _0)]
    UnsupportedProtocol(String),
}

impl From<IoError> for ServerStartError {
    fn from(e: IoError) -> Self {
        ServerStartError::IoError(e)
    }
}

impl From<TlsError> for ServerStartError {
    fn from(e: TlsError) -> Self {
        ServerStartError::TlsError(e)
    }
}

#[derive(Fail, Debug)]
pub enum ServerStopError {
    #[fail(display = "The server is already stopped")]
    AlreadyStopped,
}
