use ed25519_dalek::SignatureError;
use hex::FromHexError;

pub type KeysError = SignatureError;

#[derive(Debug, Fail)]
pub enum ParseError {
    #[fail(display = "{}", _0)]
    FromHex(#[cause] FromHexError),
    #[fail(display = "{}", _0)]
    KeysError(#[cause] KeysError),
}

impl From<FromHexError> for ParseError {
    fn from(e: FromHexError) -> Self {
        ParseError::FromHex(e)
    }
}

impl From<KeysError> for ParseError {
    fn from(e: KeysError) -> Self {
        ParseError::KeysError(e)
    }
}
