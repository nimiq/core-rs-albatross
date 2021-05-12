use thiserror::Error;

use genesis::NetworkId;
use peer_address::address::peer_uri::PeerUriError;
use utils::file_store::Error as KeyStoreError;

use crate::websocket::error::ServerStartError;

#[derive(Error, Debug)]
pub enum Error {
    #[error("PeerKey has not been initialized")]
    UninitializedPeerKey,
    #[error("{0}")]
    KeyStoreError(#[from] KeyStoreError),
    #[error("{0}")]
    ServerStartError(#[from] ServerStartError),
    #[error("Could not load network info for id {0:?}")]
    InvalidNetworkInfo(NetworkId),
    #[error("Could not add seed node {0}")]
    InvalidSeed(#[from] SeedError),
}

#[derive(Debug, Error)]
pub enum SeedError {
    #[error("Invalid peer URI: {0}")]
    Peer(#[from] PeerUriError),
    #[error("Invalid seed list URL: {0}")]
    Url(#[from] url::ParseError),
}
