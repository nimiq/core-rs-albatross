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
pub use macro_block::{MacroBlock, MacroHeader, MacroExtrinsics, ValidatorSlot};
pub use micro_block::{MicroBlock, MicroHeader, MicroJustification, MicroExtrinsics};
pub use view_change::{ViewChange, SignedViewChange, ViewChangeProof};
pub use fork_proof::ForkProof;
pub use pbft::{PbftPrepareMessage, PbftCommitMessage, PbftProof, UntrustedPbftProof, SignedPbftPrepareMessage, SignedPbftCommitMessage, SignedPbftProposal, PbftProposal};

use beserial::{Deserialize, Serialize};
use bls::bls12_381::PublicKey;
use keys::Address;

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

    DuplicateTransaction,
    InvalidTransaction(TransactionError),
    ExpiredTransaction,
    TransactionsNotOrdered,

    DuplicateReceipt,
    InvalidReceipt,
    ReceiptsNotOrdered,
}

impl block_base::BlockError for BlockError {}
