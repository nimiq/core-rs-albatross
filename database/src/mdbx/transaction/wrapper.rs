use std::ops::Deref;

use libmdbx::{NoWriteMap, RO, RW};
use nimiq_database_value::IntoDatabaseValue;

use super::MdbxTransaction;
use crate::{
    mdbx::{CursorProxy, MdbxCursor},
    traits::{DupTable, ReadTransaction, RegularTable, Table, WriteTransaction},
};

/// A proxy object that can be either a read or a write transaction.
/// This is used to the outside to hide the transaction kind generic.
pub enum MdbxReadTransaction<'db> {
    Read(MdbxTransaction<'db, RO>),
    Write(MdbxTransaction<'db, RW>),
}

impl<'db> AsRef<MdbxReadTransaction<'db>> for MdbxReadTransaction<'db> {
    fn as_ref(&self) -> &MdbxReadTransaction<'db> {
        self
    }
}

impl<'db> MdbxReadTransaction<'db> {
    pub(crate) fn new_read(txn: libmdbx::Transaction<'db, RO, NoWriteMap>) -> Self {
        MdbxReadTransaction::Read(MdbxTransaction::new(txn))
    }
}

impl<'db> ReadTransaction<'db> for MdbxReadTransaction<'db> {
    type Cursor<'txn, T: Table> = CursorProxy<'txn, T> where Self: 'txn;

    type DupCursor<'txn, T: DupTable> = CursorProxy<'txn, T> where  Self: 'txn;

    fn get<T: Table>(&self, table: &T, key: &T::Key) -> Option<T::Value> {
        match self {
            MdbxReadTransaction::Read(ref txn) => txn.get(table, key),
            MdbxReadTransaction::Write(ref txn) => txn.get(table, key),
        }
    }

    fn cursor<'txn, T: RegularTable>(&'txn self, table: &T) -> Self::Cursor<'txn, T> {
        match self {
            MdbxReadTransaction::Read(ref txn) => CursorProxy::Read(txn.cursor(table)),
            MdbxReadTransaction::Write(ref txn) => {
                CursorProxy::Write(ReadTransaction::cursor(txn, table))
            }
        }
    }

    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupCursor<'txn, T> {
        match self {
            MdbxReadTransaction::Read(ref txn) => CursorProxy::Read(txn.dup_cursor(table)),
            MdbxReadTransaction::Write(ref txn) => {
                CursorProxy::Write(ReadTransaction::dup_cursor(txn, table))
            }
        }
    }
}

pub struct MdbxWriteTransaction<'db> {
    txn: MdbxReadTransaction<'db>,
}

impl<'db> MdbxWriteTransaction<'db> {
    pub(crate) fn new(txn: libmdbx::Transaction<'db, RW, NoWriteMap>) -> Self {
        Self {
            txn: MdbxReadTransaction::Write(MdbxTransaction::new(txn)),
        }
    }
}

impl<'db> ReadTransaction<'db> for MdbxWriteTransaction<'db> {
    type Cursor<'txn, T: Table> = CursorProxy<'txn, T> where Self: 'txn;

    type DupCursor<'txn, T: DupTable> = CursorProxy<'txn, T>
    where Self: 'txn;

    fn get<T: Table>(&self, table: &T, key: &T::Key) -> Option<T::Value> {
        self.txn.get(table, key)
    }

    fn cursor<'txn, T: RegularTable>(&'txn self, table: &T) -> Self::Cursor<'txn, T> {
        self.txn.cursor(table)
    }

    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupCursor<'txn, T> {
        self.txn.dup_cursor(table)
    }
}

impl<'db> WriteTransaction<'db> for MdbxWriteTransaction<'db> {
    type WriteCursor<'txn, T: Table> = MdbxCursor<'txn, RW, T>where Self: 'txn;

    type DupWriteCursor<'txn, T: DupTable> = MdbxCursor<'txn, RW, T> where Self: 'txn;

    fn put_reserve<T: RegularTable>(&mut self, table: &T, key: &T::Key, value: &T::Value)
    where
        T::Value: IntoDatabaseValue,
    {
        match self.txn {
            MdbxReadTransaction::Write(ref mut txn) => txn.put_reserve(table, key, value),
            _ => unreachable!(),
        }
    }

    fn put<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value) {
        match self.txn {
            MdbxReadTransaction::Write(ref mut txn) => txn.put(table, key, value),
            _ => unreachable!(),
        }
    }

    fn append<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value) {
        match self.txn {
            MdbxReadTransaction::Write(ref mut txn) => txn.append(table, key, value),
            _ => unreachable!(),
        }
    }

    fn remove<T: Table>(&mut self, table: &T, key: &T::Key) {
        match self.txn {
            MdbxReadTransaction::Write(ref mut txn) => txn.remove(table, key),
            _ => unreachable!(),
        }
    }

    fn remove_item<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value) {
        match self.txn {
            MdbxReadTransaction::Write(ref mut txn) => txn.remove_item(table, key, value),
            _ => unreachable!(),
        }
    }

    fn commit(self) {
        match self.txn {
            MdbxReadTransaction::Write(txn) => txn.commit(),
            _ => unreachable!(),
        }
    }

    fn cursor<'txn, T: RegularTable>(&'txn self, table: &T) -> Self::WriteCursor<'txn, T> {
        match self.txn {
            MdbxReadTransaction::Write(ref txn) => WriteTransaction::cursor(txn, table),
            _ => unreachable!(),
        }
    }

    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupWriteCursor<'txn, T> {
        match self.txn {
            MdbxReadTransaction::Write(ref txn) => WriteTransaction::dup_cursor(txn, table),
            _ => unreachable!(),
        }
    }

    fn clear_table<T: Table>(&mut self, table: &T) {
        match self.txn {
            MdbxReadTransaction::Write(ref mut txn) => txn.clear_table(table),
            _ => unreachable!(),
        }
    }
}

impl<'db> Deref for MdbxWriteTransaction<'db> {
    type Target = MdbxReadTransaction<'db>;

    fn deref(&self) -> &Self::Target {
        &self.txn
    }
}

impl<'db> AsRef<MdbxReadTransaction<'db>> for MdbxWriteTransaction<'db> {
    fn as_ref(&self) -> &MdbxReadTransaction<'db> {
        &self.txn
    }
}
