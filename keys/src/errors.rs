pub use ed25519_zebra::{ed25519::Error as SignatureError, Error as KeysError};
use hex::FromHexError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{0}")]
    FromHex(#[from] FromHexError),
    #[error("{0}")]
    KeysError(#[from] KeysError),
    #[error("{0}")]
    SignatureError(#[from] SignatureError),
}
