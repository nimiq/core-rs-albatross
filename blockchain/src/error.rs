use thiserror::Error;

use nimiq_account::AccountError;
use nimiq_block::{Block, BlockError, ForkProof};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::networks::NetworkId;

/// An enum used when a fork is detected.
#[derive(Clone, Debug)]
pub enum ForkEvent {
    Detected(ForkProof),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockchainEvent {
    Extended(Blake2bHash),
    HistoryAdopted(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, Block)>, Vec<(Blake2bHash, Block)>),
    Finalized(Blake2bHash),
    EpochFinalized(Blake2bHash),
}

impl BlockchainEvent {
    pub fn get_newest_hash(&self) -> Blake2bHash {
        *match self {
            Self::Extended(h) => h,
            Self::HistoryAdopted(h) => h,
            Self::Rebranched(_, new_chain) => {
                if let Some((h, _)) = new_chain.last() {
                    h
                } else {
                    unreachable!()
                }
            }
            Self::Finalized(h) => h,
            Self::EpochFinalized(h) => h,
        }
    }
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
    #[error("Block not found")]
    BlockNotFound,
    #[error("Block body not found")]
    BlockBodyNotFound,
    #[error("Block is not a macro block")]
    BlockIsNotMacro,
    #[error("No validators found")]
    NoValidatorsFound,
    #[error("Invalid epoch ID")]
    InvalidEpoch,
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
