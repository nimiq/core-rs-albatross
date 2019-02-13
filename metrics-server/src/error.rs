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
