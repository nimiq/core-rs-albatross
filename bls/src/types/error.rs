use hex::FromHexError;
use thiserror::Error;

// TODO: These both are identical
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Not valid hexadecimal: {0}")]
    InvalidHex(#[from] FromHexError),
    #[error("Incorrect length: {}", 0)]
    IncorrectLength(usize),
    #[error("Serialization error")]
    SerializationError,
}
