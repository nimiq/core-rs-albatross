use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};

use super::{ReadCursor, WriteCursor};

/// Read-transactions can only perform read operations on a database.
pub trait ReadTransaction<'db>: Sized {
    type Table;
    type Cursor<'txn>: ReadCursor<'txn>
    where
        Self: 'txn;

    fn close(self) {}

    fn get<K, V>(&self, table: &Self::Table, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue;

    fn cursor<'txn>(&'txn self, table: &Self::Table) -> Self::Cursor<'txn>;
}

/// Write-transactions can perform read and write operations on a database.
pub trait WriteTransaction<'db>: ReadTransaction<'db> + Sized {
    type WriteCursor<'txn>: WriteCursor<'txn>
    where
        Self: 'txn;

    /// Puts a key/value pair into the database by copying it into a reserved space in the database.
    /// This works best for values that need to be serialized into the reserved space.
    /// This method will panic when called on a database with duplicate keys!
    fn put_reserve<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: IntoDatabaseValue + ?Sized;

    /// Puts a key/value pair into the database by passing a reference to a byte slice.
    /// This is more efficient than `put_reserve` if no serialization is needed,
    /// and the existing value can be immediately written into the database.
    /// This also works with duplicate key databases.
    fn put<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    /// Appends a key/value pair to the end of the database.
    /// This operation fails if the key is less than the last key.
    fn append<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    fn remove<K>(&mut self, table: &Self::Table, key: &K)
    where
        K: AsDatabaseBytes + ?Sized;

    fn remove_item<K, V>(&mut self, table: &Self::Table, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    fn commit(self);

    fn abort(self) {}

    fn cursor<'txn>(&'txn self, table: &Self::Table) -> Self::WriteCursor<'txn>;

    fn clear_database(&mut self, table: &Self::Table);
}
