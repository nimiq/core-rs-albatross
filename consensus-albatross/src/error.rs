use failure::Fail;

use blockchain_albatross::BlockchainError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    BlockchainError(#[cause] BlockchainError),
}

impl From<BlockchainError> for Error {
    fn from(e: BlockchainError) -> Self {
        Error::BlockchainError(e)
    }
}

#[derive(Fail, Debug)]
pub enum SyncError {
    #[fail(display = "Other")]
    Other,
    #[fail(display = "No valid sync target found")]
    NoValidSyncTarget,
}
