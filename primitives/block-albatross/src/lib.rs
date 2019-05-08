#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate nimiq_hash_derive as hash_derive;
extern crate nimiq_account as account;
extern crate nimiq_bls as bls;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;
extern crate nimiq_collections as collections;

mod block;
mod macro_block;
mod micro_block;
mod pbft;
mod slash;
mod view_change;
pub mod signed;

pub use block::{Block, BlockHeader};
pub use macro_block::{MacroBlock, MacroHeader};
pub use micro_block::{MicroBlock, MicroHeader, MicroExtrinsics};
pub use view_change::{ViewChange, SignedViewChange, ViewChangeProof};
pub use slash::SlashInherent;
pub use pbft::{PbftPrepareMessage, PbftCommitMessage, PbftProof, SignedPbftPrepareMessage,
               SignedPbftCommitMessage, SignedPbftProposal, PbftProposal};

use crate::transaction::TransactionError;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlockError {
    UnsupportedVersion,
    FromTheFuture,
    SizeExceeded,
    BodyHashMismatch,

    DuplicateTransaction,
    InvalidTransaction(TransactionError),
    ExpiredTransaction,
    TransactionsNotOrdered,

    DuplicateAccountReceipt,
    AccountReceiptsNotOrdered,
    InvalidAccountReceipt,
}
