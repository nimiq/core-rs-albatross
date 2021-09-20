#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;

use thiserror::Error;

pub use block::*;
pub use fork_proof::*;
pub use macro_block::*;
pub use micro_block::*;
pub use multisig::*;
use nimiq_transaction::TransactionError;
pub use signed::*;
pub use tendermint::*;
pub use view_change::*;

mod block;
mod fork_proof;
mod macro_block;
mod micro_block;
mod multisig;
mod signed;
mod tendermint;
mod view_change;

/// Enum containing a variety of block error types.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum BlockError {
    #[error("Unsupported version")]
    UnsupportedVersion,
    #[error("Block is from the future")]
    FromTheFuture,
    #[error("Block size exceeded")]
    SizeExceeded,

    #[error("Body hash mismatch")]
    BodyHashMismatch,
    #[error("Accounts hash mismatch")]
    AccountsHashMismatch,

    #[error("Missing justification")]
    NoJustification,
    #[error("Missing view change proof")]
    NoViewChangeProof,
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
    #[error("Transactions incorrectly ordered")]
    TransactionsNotOrdered,

    #[error("Duplicate receipt in block")]
    DuplicateReceipt,
    #[error("Invalid receipt in block")]
    InvalidReceipt,
    #[error("Receipts incorrectly ordered")]
    ReceiptsNotOrdered,

    #[error("Justification is invalid")]
    InvalidJustification,
    #[error("View change proof is invalid")]
    InvalidViewChangeProof,
    #[error("Contains an invalid seed")]
    InvalidSeed,
    #[error("Invalid view number")]
    InvalidViewNumber,
    #[error("Invalid history root")]
    InvalidHistoryRoot,
    #[error("Incorrect validators")]
    InvalidValidators,
}
