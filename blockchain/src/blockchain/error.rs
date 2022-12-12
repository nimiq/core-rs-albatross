use thiserror::Error;

use nimiq_account::AccountError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunksPushResult {
    NoChunks,
    Chunks,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ChunksPushError {
    #[error("Account error in chunk {0}: {1}")]
    AccountsError(usize, AccountError),
}

impl ChunksPushError {
    pub fn chunk_index(&self) -> usize {
        match self {
            ChunksPushError::AccountsError(i, _) => *i,
        }
    }
}
