#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;
extern crate nimiq_account as account;
extern crate nimiq_block_base as block_base;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
#[macro_use]
extern crate nimiq_macros as macros;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

mod block;
mod body;
mod header;
mod interlink;
mod target;
pub mod proof;

pub use self::block::Block;
pub use self::body::BlockBody;
pub use self::header::BlockHeader;
pub use self::interlink::BlockInterlink;
pub use self::target::{Target, TargetCompact, Difficulty};

use crate::transaction::TransactionError;

#[derive(Clone, PartialEq, Eq, Debug, Fail)]
pub enum BlockError {
    #[fail(display = "Unsupported version")]
    UnsupportedVersion,
    #[fail(display = "Block is from the future")]
    FromTheFuture,
    #[fail(display = "Invalid proof of work")]
    InvalidPoW,
    #[fail(display = "Block size exceeded")]
    SizeExceeded,
    #[fail(display = "Interlink hash mismatch")]
    InterlinkHashMismatch,
    #[fail(display = "Body hash mismatch")]
    BodyHashMismatch,
    #[fail(display = "Accounts hash mismatch")]
    AccountsHashMismatch,

    #[fail(display = "Duplicate transaction in block")]
    DuplicateTransaction,
    #[fail(display = "Invalid transaction in block: {}", _0)]
    InvalidTransaction(#[cause] TransactionError),
    #[fail(display = "Expired transaction in block")]
    ExpiredTransaction,
    #[fail(display = "Transactions incorrectly ordered")]
    TransactionsNotOrdered,
    #[fail(display = "Fee overflow")]
    FeeOverflow,

    #[fail(display = "Duplicate receipt in block")]
    DuplicateReceipt,
    #[fail(display = "Invalid receipt in block")]
    InvalidReceipt,
    #[fail(display = "Receipts incorrectly ordered")]
    ReceiptsNotOrdered,
}

impl block_base::BlockError for BlockError {}
