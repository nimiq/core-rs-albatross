#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate nimiq_hash_derive as hash_derive;
extern crate nimiq_account as account;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

mod block;
mod macro_block;
mod micro_block;
mod pbft;
mod slash;
mod view_change;
pub mod signed;

pub use block::Block;
pub use macro_block::{MacroBlock, MacroHeader, MacroExtrinsics};
pub use micro_block::{MicroBlock, MicroHeader, MicroExtrinsics};
pub use view_change::{ViewChange, SignedViewChange};
pub use slash::SlashInherent;

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

    DuplicatePrunedAccount,
    PrunedAccountsNotOrdered,
    InvalidPrunedAccount,
}
