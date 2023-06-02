mod cursor;
mod database;
mod transaction;

pub use cursor::*;
pub use database::*;
pub use transaction::*;

use crate::{
    mdbx::MdbxTable,
    traits::{ReadTransaction, WriteTransaction},
};

pub type TableProxy = MdbxTable;

pub type Cursor<'txn> = <TransactionProxy<'txn> as ReadTransaction<'txn>>::Cursor<'txn>;
pub type WriteCursor<'txn> =
    <WriteTransactionProxy<'txn> as WriteTransaction<'txn>>::WriteCursor<'txn>;
