use thiserror::Error;

use nimiq_account::AccountError;
use nimiq_block::{Block, BlockError, ForkProof};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::networks::NetworkId;

/// An enum used when a fork is detected.
#[derive(Clone)]
pub enum ForkEvent {
    Detected(ForkProof),
}

/// An enum representing different types of errors associated with slashing.
#[derive(Error, Debug)]
pub enum SlashPushError {
    #[error("Redundant fork proofs in block")]
    DuplicateForkProof,
    #[error("Block contains fork proof targeting a slot that was already slashed")]
    SlotAlreadySlashed,
    #[error("Fork proof is from a wrong epoch")]
    InvalidEpochTarget,
    #[error("Fork proof infos don't match fork proofs")]
    InvalidForkProofInfos,
    #[error("Fork proof infos cannot be fetched (predecessor does not exist)")]
    InvalidForkProofPredecessor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockchainEvent {
    Extended(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, Block)>, Vec<(Blake2bHash, Block)>),
    Finalized(Blake2bHash),
    EpochFinalized(Blake2bHash),
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
