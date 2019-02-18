use failure::Fail;
use hyper::Error as HyperError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    HyperError(#[cause] HyperError),
}

impl From<HyperError> for Error {
    fn from(e: HyperError) -> Self {
        Error::HyperError(e)
    }
}

#[derive(Debug, Fail)]
pub enum AuthenticationError {
    #[fail(display = "Invalid authorization header.")]
    InvalidHeader,
    #[fail(display = "Incorrect credentials.")]
    IncorrectCredentials,
}
