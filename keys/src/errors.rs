use ed25519_zebra::Error;
use hex::FromHexError;
use thiserror::Error;

pub type KeysError = Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{0}")]
    FromHex(#[from] FromHexError),
    #[error("{0}")]
    KeysError(#[from] KeysError),
}
