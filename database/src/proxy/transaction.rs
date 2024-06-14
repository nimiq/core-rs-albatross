use std::ops::Deref;

use crate::{
    mdbx::{MdbxReadTransaction, MdbxWriteCursor, MdbxWriteTransaction},
    traits::{ReadTransaction, WriteTransaction},
    CursorProxy, TableProxy,
};

/// A transaction handle for read-only transactions.
/// Read-write transactions can be dereferenced into this type.
#[derive(Debug)]
pub enum TransactionProxy<'db> {
    ReadTransaction(MdbxReadTransaction<'db>),
    WriteTransaction(MdbxWriteTransaction<'db>),
}

/// A transaction handle for read-write transactions.
#[derive(Debug)]
pub struct WriteTransactionProxy<'db> {
    txn: TransactionProxy<'db>,
}

impl<'db> ReadTransaction<'db> for TransactionProxy<'db> {
    type Table = TableProxy;

    type Cursor<'txn> = CursorProxy<'txn>
    where
        Self: 'txn;

    fn get<K, V>(&self, table: &Self::Table, key: &K) -> Option<V>
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            TransactionProxy::ReadTransaction(txn) => txn.get(table, key),
            TransactionProxy::WriteTransaction(txn) => txn.get(table, key),
        }
    }

    fn cursor<'txn>(&'txn self, table: &Self::Table) -> Self::Cursor<'txn> {
        match self {
            TransactionProxy::ReadTransaction(txn) => CursorProxy::ReadCursor(txn.cursor(table)),
            TransactionProxy::WriteTransaction(txn) => {
                CursorProxy::WriteCursor(ReadTransaction::cursor(txn, table))
            }
        }
    }
}

impl<'db> WriteTransactionProxy<'db> {
    pub(super) fn new(txn: MdbxWriteTransaction<'db>) -> Self {
        Self {
            txn: TransactionProxy::WriteTransaction(txn),
        }
    }
}

impl<'db> Deref for WriteTransactionProxy<'db> {
    type Target = TransactionProxy<'db>;

    fn deref(&self) -> &Self::Target {
        &self.txn
    }
}

impl<'db> ReadTransaction<'db> for WriteTransactionProxy<'db> {
    type Table = TableProxy;

    type Cursor<'txn> = CursorProxy<'txn>
    where
        Self: 'txn;

    fn get<K, V>(&self, table: &Self::Table, key: &K) -> Option<V>
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
        V: nimiq_database_value::FromDatabaseValue,
    {
        self.txn.get(table, key)
    }

    fn cursor<'txn>(&'txn self, table: &Self::Table) -> Self::Cursor<'txn> {
        self.txn.cursor(table)
    }
}

impl<'db> WriteTransaction<'db> for WriteTransactionProxy<'db> {
    type WriteCursor<'txn> = MdbxWriteCursor<'txn>
    where
        Self: 'txn;

    fn put_reserve<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
        V: nimiq_database_value::IntoDatabaseValue + ?Sized,
    {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(ref mut txn) => txn.put_reserve(table, key, value),
        }
    }

    fn put<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
        V: nimiq_database_value::AsDatabaseBytes + ?Sized,
    {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(ref mut txn) => txn.put(table, key, value),
        }
    }

    fn append<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
        V: nimiq_database_value::AsDatabaseBytes + ?Sized,
    {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(ref mut txn) => txn.append(table, key, value),
        }
    }

    fn remove<K>(&mut self, table: &Self::Table, key: &K)
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
    {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(ref mut txn) => txn.remove(table, key),
        }
    }

    fn remove_item<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
        V: nimiq_database_value::AsDatabaseBytes + ?Sized,
    {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(ref mut txn) => txn.remove_item(table, key, value),
        }
    }

    fn commit(self) {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(txn) => txn.commit(),
        }
    }

    fn cursor<'txn>(&'txn self, table: &Self::Table) -> Self::WriteCursor<'txn> {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(ref txn) => WriteTransaction::cursor(txn, table),
        }
    }

    fn clear_database(&mut self, table: &Self::Table) {
        match self.txn {
            TransactionProxy::ReadTransaction(_) => unreachable!(),
            TransactionProxy::WriteTransaction(ref mut txn) => txn.clear_database(table),
        }
    }
}
