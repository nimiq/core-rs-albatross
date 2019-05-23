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
pub use macro_block::{MacroBlock, MacroHeader, MacroExtrinsics};
pub use micro_block::{MicroBlock, MicroHeader, MicroJustification, MicroExtrinsics};
pub use view_change::{ViewChange, SignedViewChange, ViewChangeProof, ViewChangeProofBuilder, ViewChanges};
pub use fork_proof::ForkProof;
pub use pbft::{PbftPrepareMessage, PbftCommitMessage, PbftProofBuilder, PbftProof, SignedPbftPrepareMessage, SignedPbftCommitMessage, SignedPbftProposal, PbftProposal};

use crate::transaction::TransactionError;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlockError {
    UnsupportedVersion,
    FromTheFuture,
    SizeExceeded,
    BodyHashMismatch,
    AccountsHashMismatch,
    NoJustification,
    NoViewChangeProof,

    InvalidForkProof,
    DuplicateForkProof,
    ForkProofsNotOrdered,

    InvalidTransaction(TransactionError),
    DuplicateTransaction,
    TransactionsNotOrdered,
    ExpiredTransaction,

    DuplicateReceipt,
    InvalidReceipt,
    ReceiptsNotOrdered,

    InvalidJustification,
    InvalidSlash,
}

impl block_base::BlockError for BlockError {}

impl From<signed::AggregateProofError> for BlockError {
    fn from(_e: signed::AggregateProofError) -> Self {
        BlockError::InvalidJustification
    }
}
