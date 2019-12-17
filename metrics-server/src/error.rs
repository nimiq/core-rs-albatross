use failure::Fail;
use std::io::Error as IoError;
use native_tls::Error as NativeTlsError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    IoError(#[cause] IoError),
    #[fail(display = "{}", _0)]
    NativeTlsError(#[cause] NativeTlsError),
    #[fail(display = "{}", _0)]
    HyperError(#[cause] hyper::Error),
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

impl From<hyper::Error> for Error {
    fn from(e: hyper::Error) -> Self {
        Error::HyperError(e)
    }
}
