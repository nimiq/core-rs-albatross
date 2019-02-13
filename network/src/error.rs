use failure::Fail;
use tokio::io::Error as IoError;

use network_primitives::networks::NetworkId;

use crate::websocket::error::ServerStartError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "PeerKey could not be deserialized")]
    PeerKeyInvalid,
    #[fail(display = "PeerKey has not been initialized")]
    PeerKeyUninitialized,
    #[fail(display = "{}", _0)]
    IoError(#[cause] IoError),
    #[fail(display = "{}", _0)]
    ServerStartError(#[cause] ServerStartError),
    #[fail(display = "Could not load network info for id {:?}", _0)]
    InvalidNetworkInfo(NetworkId),
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Self {
        Error::IoError(e)
    }
}

impl From<ServerStartError> for Error {
    fn from(e: ServerStartError) -> Self {
        Error::ServerStartError(e)
    }
}
