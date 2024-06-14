use std::borrow::Cow;

use libmdbx::{NoWriteMap, TransactionKind, WriteFlags, RO, RW};
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};

use super::{MdbxCursor, MdbxTable, MdbxWriteCursor};
use crate::traits::{ReadTransaction, WriteTransaction};

/// Wrapper around mdbx transactions that only exposes our own traits.
#[derive(Debug)]
pub struct MdbxTransaction<'db, K: TransactionKind> {
    txn: libmdbx::Transaction<'db, K, NoWriteMap>,
}
/// Instantiation for read-only transactions.
pub type MdbxReadTransaction<'db> = MdbxTransaction<'db, RO>;
/// Instantiation for read-write transactions.
pub type MdbxWriteTransaction<'db> = MdbxTransaction<'db, RW>;

impl<'db, Kind> MdbxTransaction<'db, Kind>
where
    Kind: TransactionKind,
{
    pub(crate) fn new(txn: libmdbx::Transaction<'db, Kind, NoWriteMap>) -> Self {
        MdbxTransaction { txn }
    }

    pub(super) fn open_table(&self, table: &MdbxTable) -> libmdbx::Table {
        self.txn.open_table(Some(&table.name)).unwrap()
    }
}

impl<'db, Kind> ReadTransaction<'db> for MdbxTransaction<'db, Kind>
where
    Kind: TransactionKind,
{
    type Table = MdbxTable;
    type Cursor<'txn> = MdbxCursor<'txn, Kind> where 'db: 'txn;

    fn get<K, V>(&self, table: &MdbxTable, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let table = self.open_table(table);

        let result: Option<Cow<[u8]>> = self
            .txn
            .get(&table, &AsDatabaseBytes::as_database_bytes(key))
            .unwrap();

        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn cursor<'txn>(&'txn self, table: &MdbxTable) -> MdbxCursor<'txn, Kind> {
        let table = self.open_table(table);

        MdbxCursor::new(self.txn.cursor(&table).unwrap())
    }
}

impl<'db> WriteTransaction<'db> for MdbxWriteTransaction<'db> {
    type WriteCursor<'txn> = MdbxWriteCursor<'txn> where 'db: 'txn;

    fn put_reserve<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: IntoDatabaseValue + ?Sized,
    {
        let table = self.open_table(table);

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value_size = IntoDatabaseValue::database_byte_size(value);

        let bytes: &mut [u8] = self
            .txn
            .reserve(&table, key, value_size, WriteFlags::empty())
            .unwrap();

        IntoDatabaseValue::copy_into_database(value, bytes);
    }

    fn put<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let table = self.open_table(table);

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);

        self.txn
            .put(&table, key, value, WriteFlags::empty())
            .unwrap();
    }

    fn append<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let table = self.open_table(table);

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);

        self.txn
            .put(&table, key, value, WriteFlags::APPEND)
            .unwrap();
    }

    fn remove<K>(&mut self, table: &MdbxTable, key: &K)
    where
        K: AsDatabaseBytes + ?Sized,
    {
        let table = self.open_table(table);

        self.txn
            .del(
                &table,
                AsDatabaseBytes::as_database_bytes(key).as_ref(),
                None,
            )
            .unwrap();
    }

    fn remove_item<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let table = self.open_table(table);

        self.txn
            .del(
                &table,
                AsDatabaseBytes::as_database_bytes(key).as_ref(),
                Some(AsDatabaseBytes::as_database_bytes(value).as_ref()),
            )
            .unwrap();
    }

    fn commit(self) {
        self.txn.commit().unwrap();
    }

    fn cursor<'txn>(&'txn self, table: &MdbxTable) -> MdbxWriteCursor<'txn> {
        let table = self.open_table(table);

        MdbxWriteCursor::new(self.txn.cursor(&table).unwrap())
    }

    fn clear_database(&mut self, table: &MdbxTable) {
        let table = self.open_table(table);

        self.txn.clear_table(&table).unwrap();
    }
}
