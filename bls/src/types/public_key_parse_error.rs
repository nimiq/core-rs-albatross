use super::*;

#[derive(Clone, Debug, Fail)]
pub enum PublicKeyParseError {
    #[fail(display = "Not valid hexadecimal: {}", _0)]
    InvalidHex(#[cause] FromHexError),
    #[fail(display = "Incorrect length: {}", _0)]
    IncorrectLength(usize),
}

impl From<FromHexError> for PublicKeyParseError {
    fn from(e: FromHexError) -> Self {
        PublicKeyParseError::InvalidHex(e)
    }
}
