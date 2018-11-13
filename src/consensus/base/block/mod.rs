mod block;
mod body;
mod header;
mod interlink;
mod target;

pub use self::block::Block;
pub use self::body::BlockBody;
pub use self::header::BlockHeader;
pub use self::interlink::BlockInterlink;
pub use self::target::{Target, TargetCompact, Difficulty};

use consensus::base::transaction::TransactionError;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
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
