use failure::Fail;

use account::AccountError;
use block::{Block, BlockError, ForkProof};
use hash::Blake2bHash;
use primitives::networks::NetworkId;

/// An enum used when a fork is detected.
#[derive(Clone)]
pub enum ForkEvent {
    Detected(ForkProof),
}

/// An enum representing different types of errors associated with slashing.
#[derive(Debug, Fail)]
pub enum SlashPushError {
    #[fail(display = "Redundant fork proofs in block")]
    DuplicateForkProof,
    #[fail(display = "Block contains fork proof targeting a slot that was already slashed")]
    SlotAlreadySlashed,
    #[fail(display = "Fork proof is from a wrong epoch")]
    InvalidEpochTarget,
    #[fail(display = "Fork proof infos don't match fork proofs")]
    InvalidForkProofInfos,
    #[fail(display = "Fork proof infos cannot be fetched (predecessor does not exist)")]
    InvalidForkProofPredecessor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockchainEvent {
    Extended(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, Block)>, Vec<(Blake2bHash, Block)>),
    Finalized(Blake2bHash),
    EpochFinalized(Blake2bHash),
}

#[derive(Debug, Fail, Clone, PartialEq, Eq)]
pub enum BlockchainError {
    #[fail(display = "Invalid genesis block stored. Are you on the right network?")]
    InvalidGenesisBlock,
    #[fail(display = "Failed to load the main chain. Reset your consensus database.")]
    FailedLoadingMainChain,
    #[fail(display = "Inconsistent chain/accounts state. Reset your consensus database.")]
    InconsistentState,
    #[fail(display = "No network for: {:?}", _0)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    Orphan,
    InvalidBlock(BlockError),
    InvalidSuccessor,
    DuplicateTransaction,
    AccountsError(AccountError),
    InvalidFork,
    BlockchainError(BlockchainError),
}

impl PushError {
    /// Create a `PushError` from a `BlockError`.
    ///
    /// NOTE: We can't implement `From<BE: BlockError>`, since the compiler can't guarantee that
    /// nobody will implement `BlockError` for `AccountError`, which would result in a duplicate
    /// implementation.
    pub fn from_block_error(e: BlockError) -> Self {
        PushError::InvalidBlock(e)
    }
}

impl From<AccountError> for PushError {
    fn from(e: AccountError) -> Self {
        PushError::AccountsError(e)
    }
}

impl From<BlockchainError> for PushError {
    fn from(e: BlockchainError) -> Self {
        PushError::BlockchainError(e)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Forward,
    Backward,
}
