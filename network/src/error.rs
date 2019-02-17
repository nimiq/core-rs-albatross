use failure::Fail;
use tokio::io::Error as IoError;

use network_primitives::networks::NetworkId;
use network_primitives::address::peer_uri::PeerUriError;

use crate::websocket::error::ServerStartError;


#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "PeerKey could not be deserialized")]
    InvalidPeerKey,
    #[fail(display = "PeerKey has not been initialized")]
    UninitializedPeerKey,
    #[fail(display = "{}", _0)]
    IoError(#[cause] IoError),
    #[fail(display = "{}", _0)]
    ServerStartError(#[cause] ServerStartError),
    #[fail(display = "Could not load network info for id {:?}", _0)]
    InvalidNetworkInfo(NetworkId),
    #[fail(display = "Could not add seed node {}", _0)]
    InvalidSeed(#[cause] SeedError)
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

impl From<SeedError> for Error {
    fn from(e: SeedError) -> Self {
        Error::InvalidSeed(e)
    }
}

#[derive(Debug, Fail)]
pub enum SeedError {
    #[fail(display = "Invalid peer URI: {}", _0)]
    Peer(#[cause] PeerUriError),
    #[fail(display = "Invalid seed list URL: {}", _0)]
    Url(#[cause] url::ParseError)
}

impl From<PeerUriError> for SeedError {
    fn from(e: PeerUriError) -> SeedError {
        SeedError::Peer(e)
    }
}

impl From<url::ParseError> for SeedError {
    fn from(e: url::ParseError) -> SeedError {
        SeedError::Url(e)
    }
}
