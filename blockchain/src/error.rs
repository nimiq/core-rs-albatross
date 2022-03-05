use thiserror::Error;

use nimiq_account::AccountError;
use nimiq_block::{Block, BlockError, ForkProof};
use nimiq_hash::Blake3Hash;
use nimiq_primitives::networks::NetworkId;

/// An enum used when a fork is detected.
#[derive(Clone)]
pub enum ForkEvent {
    Detected(ForkProof),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockchainEvent {
    Extended(Blake3Hash),
    Rebranched(Vec<(Blake3Hash, Block)>, Vec<(Blake3Hash, Block)>),
    Finalized(Blake3Hash),
    EpochFinalized(Blake3Hash),
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BlockchainError {
    #[error("Invalid genesis block stored. Are you on the right network?")]
    InvalidGenesisBlock,
    #[error("Failed to load the main chain. Reset your consensus database.")]
    FailedLoadingMainChain,
    #[error("Inconsistent chain/accounts state. Reset your consensus database.")]
    InconsistentState,
    #[error("No network for: {:?}", _0)]
    NoNetwork(NetworkId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
    Known,
    Extended,
    Rebranched,
    Forked,
    Ignored,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum PushError {
    #[error("Orphan block")]
    Orphan,
    #[error("Invalid zk proof")]
    InvalidZKP,
    #[error("Invalid block: {0}")]
    InvalidBlock(#[from] BlockError),
    #[error("Invalid successor")]
    InvalidSuccessor,
    #[error("Invalid predecessor")]
    InvalidPredecessor,
    #[error("Duplicate transaction")]
    DuplicateTransaction,
    #[error("Account error: {0}")]
    AccountsError(#[from] AccountError),
    #[error("Invalid fork")]
    InvalidFork,
    #[error("Blockchain error: {0}")]
    BlockchainError(#[from] BlockchainError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Forward,
    Backward,
}
