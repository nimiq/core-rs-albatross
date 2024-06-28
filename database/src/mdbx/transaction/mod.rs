mod proxy;
mod wrapper;

use std::borrow::Cow;

use libmdbx::{NoWriteMap, WriteFlags};
pub use libmdbx::{TransactionKind, RO, RW};
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes, IntoDatabaseValue};
pub use proxy::*;
pub use wrapper::*;

use super::{MdbxCursor, MdbxWriteCursor};
use crate::traits::{DupTable, ReadTransaction, Table, WriteTransaction};

/// Wrapper around mdbx transactions that only exposes our own methods.
#[derive(Debug)]
pub struct MdbxTransaction<'db, K: TransactionKind> {
    txn: libmdbx::Transaction<'db, K, NoWriteMap>,
}

impl<'db, Kind> MdbxTransaction<'db, Kind>
where
    Kind: TransactionKind,
{
    pub(crate) fn new(txn: libmdbx::Transaction<'db, Kind, NoWriteMap>) -> Self {
        MdbxTransaction { txn }
    }

    pub(super) fn open_table<T: Table>(&self, _table: &T) -> libmdbx::Table {
        self.txn.open_table(Some(T::NAME)).unwrap()
    }
}

impl<'db, Kind> ReadTransaction<'db> for MdbxTransaction<'db, Kind>
where
    Kind: TransactionKind,
{
    type Cursor<'txn, T: Table> = MdbxCursor<'txn, Kind, T> where 'db: 'txn;

    type DupCursor<'txn, T: DupTable> = MdbxCursor<'txn, Kind, T> where 'db: 'txn;

    fn get<T: Table>(&self, table: &T, key: &T::Key) -> Option<T::Value> {
        let table = self.open_table(table);

        let result: Option<Cow<[u8]>> = self
            .txn
            .get(&table, &AsDatabaseBytes::as_key_bytes(key))
            .unwrap();

        Some(FromDatabaseBytes::from_value_bytes(&result?))
    }

    fn cursor<'txn, T: Table>(&'txn self, table: &T) -> Self::Cursor<'txn, T> {
        let table = self.open_table(table);

        MdbxCursor::new(self.txn.cursor(&table).unwrap())
    }

    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupCursor<'txn, T> {
        let table = self.open_table(table);

        MdbxCursor::new(self.txn.cursor(&table).unwrap())
    }
}

impl<'db> WriteTransaction<'db> for MdbxTransaction<'db, RW> {
    type WriteCursor<'txn, T: Table> = MdbxWriteCursor<'txn, T> where 'db: 'txn;

    type DupWriteCursor<'txn, T: DupTable> = MdbxWriteCursor<'txn, T> where 'db: 'txn;

    fn put_reserve<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value)
    where
        T::Value: IntoDatabaseValue,
    {
        let table = self.open_table(table);

        let key = AsDatabaseBytes::as_key_bytes(key);
        let value_size = IntoDatabaseValue::database_byte_size(value);

        let bytes: &mut [u8] = self
            .txn
            .reserve(&table, key, value_size, WriteFlags::empty())
            .unwrap();

        IntoDatabaseValue::copy_into_database(value, bytes);
    }

    fn put<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value) {
        let table = self.open_table(table);

        let key = AsDatabaseBytes::as_key_bytes(key);
        let value = AsDatabaseBytes::as_value_bytes(value);

        self.txn
            .put(&table, key, value, WriteFlags::empty())
            .unwrap();
    }

    fn append<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value) {
        let table = self.open_table(table);

        let key = AsDatabaseBytes::as_key_bytes(key);
        let value = AsDatabaseBytes::as_value_bytes(value);

        self.txn
            .put(&table, key, value, WriteFlags::APPEND)
            .unwrap();
    }

    fn remove<T: Table>(&mut self, table: &T, key: &T::Key) {
        let table = self.open_table(table);

        self.txn
            .del(&table, AsDatabaseBytes::as_key_bytes(key).as_ref(), None)
            .unwrap();
    }

    fn remove_item<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value) {
        let table = self.open_table(table);

        self.txn
            .del(
                &table,
                AsDatabaseBytes::as_key_bytes(key).as_ref(),
                Some(AsDatabaseBytes::as_value_bytes(value).as_ref()),
            )
            .unwrap();
    }

    fn commit(self) {
        self.txn.commit().unwrap();
    }

    fn cursor<'txn, T: Table>(&'txn self, table: &T) -> MdbxWriteCursor<'txn, T> {
        let table = self.open_table(table);

        MdbxWriteCursor::new(self.txn.cursor(&table).unwrap())
    }

    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupWriteCursor<'txn, T> {
        let table = self.open_table(table);

        MdbxCursor::new(self.txn.cursor(&table).unwrap())
    }

    fn clear_table<T: Table>(&mut self, table: &T) {
        let table = self.open_table(table);

        self.txn.clear_table(&table).unwrap();
    }
}
