use failure::Fail;

use blockchain_base::BlockchainError;
use consensus::Error as ConsensusError;
use utils::key_store::Error as KeyStoreError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    BlockchainError(#[cause] BlockchainError),
    #[fail(display = "{}", _0)]
    ConsensusError(#[cause] ConsensusError),
    #[fail(display = "{}", _0)]
    KeyStoreError(#[cause] KeyStoreError),
}

impl From<ConsensusError> for Error {
    fn from(e: ConsensusError) -> Self {
        Error::ConsensusError(e)
    }
}

impl From<KeyStoreError> for Error {
    fn from(e: KeyStoreError) -> Self {
        Error::KeyStoreError(e)
    }
}

impl From<BlockchainError> for Error {
    fn from(e: BlockchainError) -> Self {
        Error::BlockchainError(e)
    }
}
