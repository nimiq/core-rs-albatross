#[macro_use]
extern crate log;

pub use block::*;
pub use block_proof::*;
pub use equivocation_proof::*;
pub use macro_block::*;
pub use micro_block::*;
pub use multisig::*;
use nimiq_primitives::transaction::TransactionError;
pub use signed::*;
pub use skip_block::*;
pub use tendermint::*;
use thiserror::Error;

mod block;
mod block_proof;
mod equivocation_proof;
mod macro_block;
mod micro_block;
mod multisig;
mod signed;
mod skip_block;
mod tendermint;

/// Enum containing a variety of block error types.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum BlockError {
    #[error("Invalid block type for this block number")]
    InvalidBlockType,
    #[error("Unsupported version")]
    UnsupportedVersion,
    #[error("Invalid block number")]
    InvalidBlockNumber,
    #[error("Invalid timestamp")]
    InvalidTimestamp,
    #[error("Invalid parent hash")]
    InvalidParentHash,
    #[error("Invalid parent election hash")]
    InvalidParentElectionHash,
    #[error("Invalid or missing interlink")]
    InvalidInterlink,
    #[error("Contains an invalid seed")]
    InvalidSeed,
    #[error("Extra data too large")]
    ExtraDataTooLarge,

    #[error("Body hash mismatch")]
    BodyHashMismatch,
    #[error("Accounts hash mismatch")]
    AccountsHashMismatch,
    #[error("Invalid history root")]
    InvalidHistoryRoot,

    #[error("Block size exceeded")]
    SizeExceeded,

    #[error("Missing justification")]
    MissingJustification,
    #[error("Missing body")]
    MissingBody,

    #[error("Invalid fork proof")]
    InvalidForkProof,
    #[error("Duplicate fork proof")]
    DuplicateForkProof,
    #[error("Fork proofs incorrectly ordered")]
    ForkProofsNotOrdered,

    #[error("Duplicate transaction in block")]
    DuplicateTransaction,
    #[error("Invalid transaction in block: {}", _0)]
    InvalidTransaction(#[from] TransactionError),
    #[error("Expired transaction in block")]
    ExpiredTransaction,
    #[error("Transactions execution result mismatch")]
    TransactionExecutionMismatch,

    #[error("Justification is invalid")]
    InvalidJustification,
    #[error("Skip block proof is invalid")]
    InvalidSkipBlockProof,

    #[error("Incorrect validators")]
    InvalidValidators,

    #[error("Incorrect reward transactions")]
    InvalidRewardTransactions,

    #[error("Skip block contains a non empty body")]
    InvalidSkipBlockBody,
}
