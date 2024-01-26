use nimiq_block::{Block, BlockError, EquivocationProofError, ForkProof};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{account::AccountError, networks::NetworkId};
use nimiq_transaction::EquivocationLocator;
use thiserror::Error;

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
    /// Given Block was stored in the chain store but was not adopted as new head block.
    /// I.e. forked blocks and inferior chain blocks.
    Stored(Block),
    Finalized(Blake2bHash),
    EpochFinalized(Blake2bHash),
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BlockchainError {
    #[error("Invalid genesis block stored. Verify you are on the correct network or reset your consensus database.")]
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
    #[error("Accounts diff not found")]
    AccountsDiffNotFound,
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
    #[error("Equivocation proof: {0}")]
    InvalidEquivocationProof(#[from] EquivocationProofError),
    #[error("Account error: {0}")]
    AccountsError(#[from] AccountError),
    #[error("Invalid fork")]
    InvalidFork,
    #[error("Blockchain error: {0}")]
    BlockchainError(#[from] BlockchainError),
    #[error("Push with incomplete accounts and without trie diff")]
    MissingAccountsTrieDiff,
    #[error("Proof for equivocation already included")]
    EquivocationAlreadyIncluded(EquivocationLocator),
    #[error("Accounts trie is incomplete and thus cannot be verified.")]
    IncompleteAccountsTrie,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Forward,
    Backward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunksPushResult {
    EmptyChunks,
    /// Contains the number of committed and ignored chunks, respectively.
    Chunks(usize, usize),
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
