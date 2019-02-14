#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_account as account;
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

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlockError {
    UnsupportedVersion,
    FromTheFuture,
    InvalidPoW,
    SizeExceeded,
    InterlinkHashMismatch,
    BodyHashMismatch,

    DuplicateTransaction,
    InvalidTransaction(TransactionError),
    ExpiredTransaction,
    TransactionsNotOrdered,

    DuplicatePrunedAccount,
    PrunedAccountsNotOrdered,
    InvalidPrunedAccount,
}
