use thiserror::Error;

use blockchain::BlockchainError;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Blockchain error: {0}")]
    BlockchainError(#[from] BlockchainError),
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Other")]
    Other,
    #[error("No valid sync target found")]
    NoValidSyncTarget,
}

#[derive(Debug, Error)]
pub enum BlockQueueError {}
