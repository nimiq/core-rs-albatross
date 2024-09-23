use std::ops::Deref;

use super::MdbxReadTransaction;
use crate::{
    mdbx::CursorProxy,
    traits::{DupTable, ReadTransaction, RegularTable, Table},
};

/// A proxy object that can be either a read or a write transaction.
/// This can be used to hide the transaction kind generic.
pub enum TransactionProxy<'db, 'txn>
where
    'db: 'txn,
{
    Read(&'txn MdbxReadTransaction<'db>),
    OwnedRead(MdbxReadTransaction<'db>),
}

impl<'db, 'inner> ReadTransaction<'db> for TransactionProxy<'db, 'inner>
where
    'db: 'inner,
{
    type Cursor<'txn, T: Table>
        = CursorProxy<'txn, T>
    where
        'inner: 'txn;

    type DupCursor<'txn, T: DupTable>
        = CursorProxy<'txn, T>
    where
        'inner: 'txn;

    fn get<T: Table>(&self, table: &T, key: &T::Key) -> Option<T::Value> {
        match self {
            TransactionProxy::Read(txn) => txn.get(table, key),
            TransactionProxy::OwnedRead(ref txn) => txn.get(table, key),
        }
    }

    fn cursor<'txn, T: RegularTable>(&'txn self, table: &T) -> Self::Cursor<'txn, T> {
        match self {
            TransactionProxy::Read(txn) => txn.cursor(table),
            TransactionProxy::OwnedRead(ref txn) => txn.cursor(table),
        }
    }

    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupCursor<'txn, T> {
        match self {
            TransactionProxy::Read(txn) => txn.dup_cursor(table),
            TransactionProxy::OwnedRead(ref txn) => txn.dup_cursor(table),
        }
    }
}

impl<'db, 'inner> Deref for TransactionProxy<'db, 'inner>
where
    'db: 'inner,
{
    type Target = MdbxReadTransaction<'db>;

    fn deref(&self) -> &Self::Target {
        match self {
            TransactionProxy::Read(txn) => txn,
            TransactionProxy::OwnedRead(ref txn) => txn,
        }
    }
}

impl<'db, 'inner> AsRef<MdbxReadTransaction<'db>> for TransactionProxy<'db, 'inner>
where
    'db: 'inner,
{
    fn as_ref(&self) -> &MdbxReadTransaction<'db> {
        match self {
            TransactionProxy::Read(txn) => txn,
            TransactionProxy::OwnedRead(ref txn) => txn,
        }
    }
}
