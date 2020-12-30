use ed25519_dalek::SignatureError;
use hex::FromHexError;
use thiserror::Error;

pub type KeysError = SignatureError;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{0}")]
    FromHex(#[from] FromHexError),
    #[error("{0}")]
    KeysError(#[from] KeysError),
}
