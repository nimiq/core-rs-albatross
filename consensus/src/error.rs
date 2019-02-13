use failure::Fail;

use network::error::Error as NetworkError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    NetworkError(#[cause] NetworkError),
}

impl From<NetworkError> for Error {
    fn from(e: NetworkError) -> Self {
        Error::NetworkError(e)
    }
}
