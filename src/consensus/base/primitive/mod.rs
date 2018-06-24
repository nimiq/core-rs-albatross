use std::fmt;
use std::error;

mod hash;

#[derive(Debug, Clone, Copy)]
pub struct FromHexError;

impl fmt::Display for FromHexError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        return write!(f, "input is not valid hex");
    }
}

impl error::Error for FromHexError {
    fn description(&self) -> &str {
        return "input is not valid hex";
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

pub trait ToHex {
    fn to_hex(&self) -> String;
}

pub trait FromHex {
    fn from_hex(&self) -> Result<Vec<u8>, FromHexError>;
}

impl FromHex for str {
    fn from_hex(&self) -> Result<Vec<u8>, FromHexError> {
        unimplemented!()
    }
}

impl ToHex for [u8] {
    fn to_hex(&self) -> String {
        unimplemented!()
    }
}
