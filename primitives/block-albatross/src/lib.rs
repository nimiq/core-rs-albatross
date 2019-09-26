#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate beserial_derive;
extern crate nimiq_account as account;
extern crate nimiq_block_base as block_base;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_hash as hash;
#[macro_use]
extern crate nimiq_hash_derive as hash_derive;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

mod block;
mod macro_block;
mod micro_block;
mod pbft;
mod fork_proof;
mod view_change;
pub mod signed;

pub use block::{Block, BlockType, BlockHeader};
pub use macro_block::{MacroBlock, MacroHeader, MacroExtrinsics, SlotAddresses};
pub use micro_block::{MicroBlock, MicroHeader, MicroJustification, MicroExtrinsics};
pub use view_change::{ViewChange, SignedViewChange, ViewChangeProof, ViewChangeProofBuilder, ViewChanges};
pub use fork_proof::ForkProof;
pub use pbft::{PbftPrepareMessage, PbftCommitMessage, PbftProofBuilder, PbftProof, SignedPbftPrepareMessage, SignedPbftCommitMessage, SignedPbftProposal, PbftProposal};

use crate::transaction::TransactionError;

#[derive(Clone, PartialEq, Eq, Debug, Fail)]
pub enum BlockError {
    #[fail(display = "Unsupported version")]
    UnsupportedVersion,
    #[fail(display = "Block is from the future")]
    FromTheFuture,
    #[fail(display = "Block size exceeded")]
    SizeExceeded,
    #[fail(display = "Body hash mismatch")]
    BodyHashMismatch,
    #[fail(display = "Accounts hash mismatch")]
    AccountsHashMismatch,
    #[fail(display = "Missing justification")]
    NoJustification,
    #[fail(display = "Missing view change proof")]
    NoViewChangeProof,

    #[fail(display = "Invalid fork proof")]
    InvalidForkProof,
    #[fail(display = "Duplicate fork proof")]
    DuplicateForkProof,
    #[fail(display = "Fork proofs incorrectly ordered")]
    ForkProofsNotOrdered,


    #[fail(display = "Duplicate transaction in block")]
    DuplicateTransaction,
    #[fail(display = "Invalid transaction in block: {}", _0)]
    InvalidTransaction(TransactionError),
    #[fail(display = "Expired transaction in block")]
    ExpiredTransaction,
    #[fail(display = "Transactions incorrectly ordered")]
    TransactionsNotOrdered,

    #[fail(display = "Duplicate receipt in block")]
    DuplicateReceipt,
    #[fail(display = "Invalid receipt in block")]
    InvalidReceipt,
    #[fail(display = "Receipts incorrectly ordered")]
    ReceiptsNotOrdered,

    #[fail(display = "Justification is invalid")]
    InvalidJustification,
    #[fail(display = "Contains an invalid slash inherent")]
    InvalidSlash,
    #[fail(display = "Invalid view number")]
    InvalidViewNumber,
    #[fail(display = "Invalid transactions root")]
    InvalidTransactionsRoot,
    #[fail(display = "Incorrect validators")]
    InvalidValidators,

    #[fail(display = "Missing extrinsics")]
    MissingExtrinsics,
    #[fail(display = "Extrinsics hash mismatch")]
    ExtrinsicsHashMismatch,
}

impl block_base::BlockError for BlockError {}

impl From<signed::AggregateProofError> for BlockError {
    fn from(_e: signed::AggregateProofError) -> Self {
        BlockError::InvalidJustification
    }
}
