use std::fmt::{self, Display};
use ed25519_dalek::SignatureError;
use failure::Fail;
use hex::FromHexError;

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, Fail)]
#[fail(display = "{}", _0)]
pub struct KeysError(pub(crate) SignatureError);


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