use std::io::Error as IoError;

use failure::Fail;
use native_tls::Error as NativeTlsError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    IoError(#[cause] IoError),
    #[fail(display = "{}", _0)]
    NativeTlsError(#[cause] NativeTlsError),
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Self {
        Error::IoError(e)
    }
}

impl From<NativeTlsError> for Error {
    fn from(e: NativeTlsError) -> Self {
        Error::NativeTlsError(e)
    }
}
