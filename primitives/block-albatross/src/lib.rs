#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_account as account;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_handel as handel;
extern crate nimiq_hash as hash;
extern crate nimiq_hash_derive as hash_derive;
extern crate nimiq_keys as keys;
extern crate nimiq_nano_sync as nano_sync;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;
extern crate nimiq_vrf as vrf;

use crate::transaction::TransactionError;
use thiserror::Error;

pub use block::*;
pub use fork_proof::*;
pub use macro_block::*;
pub use micro_block::*;
pub use multisig::*;
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
