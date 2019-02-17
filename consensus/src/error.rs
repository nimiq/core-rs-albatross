use failure::Fail;

use network::error::Error as NetworkError;
use blockchain::BlockchainError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    NetworkError(#[cause] NetworkError),
    #[fail(display = "{}", _0)]
    BlockchainError(#[cause] BlockchainError),
}

impl From<NetworkError> for Error {
    fn from(e: NetworkError) -> Self {
        Error::NetworkError(e)
    }
}

impl From<BlockchainError> for Error {
    fn from(e: BlockchainError) -> Self {
        Error::BlockchainError(e)
    }
}
