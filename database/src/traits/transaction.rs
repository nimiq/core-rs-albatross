use nimiq_database_value::IntoDatabaseValue;

use super::{
    DupReadCursor, DupTable, DupWriteCursor, ReadCursor, RegularTable, Table, WriteCursor,
};

/// Read-transactions can only perform read operations on a database.
pub trait ReadTransaction<'db>: Sized {
    type Cursor<'txn, T: Table>: ReadCursor<'txn, T>
    where
        Self: 'txn;
    type DupCursor<'txn, T: DupTable>: ReadCursor<'txn, T> + DupReadCursor<'txn, T>
    where
        Self: 'txn;

    /// Close the transaction, releasing any resources it holds.
    fn close(self) {}

    /// Gets the value at a given key.
    fn get<T: Table>(&self, table: &T, key: &T::Key) -> Option<T::Value>;

    /// Creates a cursor for iterating over the table.
    fn cursor<'txn, T: RegularTable>(&'txn self, table: &T) -> Self::Cursor<'txn, T>;

    /// Creates a cursor for iterating over the table with duplicate keys.
    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupCursor<'txn, T>;
}

/// Write-transactions can perform read and write operations on a database.
pub trait WriteTransaction<'db>: ReadTransaction<'db> + Sized {
    type WriteCursor<'txn, T: Table>: WriteCursor<'txn, T>
    where
        Self: 'txn;
    type DupWriteCursor<'txn, T: DupTable>: DupWriteCursor<'txn, T>
    where
        Self: 'txn;

    /// Puts a key/value pair into the database by copying it into a reserved space in the database.
    /// This works best for values that need to be serialized into the reserved space.
    /// This method will panic when called on a database with duplicate keys!
    fn put_reserve<T: RegularTable>(&mut self, table: &T, key: &T::Key, value: &T::Value)
    where
        T::Value: IntoDatabaseValue;

    /// Puts a key/value pair into the database by passing a reference to a byte slice.
    /// This is more efficient than `put_reserve` if no serialization is needed,
    /// and the existing value can be immediately written into the database.
    fn put<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value);

    /// Appends a key/value pair to the end of the database.
    /// This method is more efficient than `put`.
    /// This operation fails if the key is less than the last key.
    fn append<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value);

    /// Removes the entry with this key from the database.
    /// In dup tables, it removes all entries with this key.
    fn remove<T: Table>(&mut self, table: &T, key: &T::Key);

    /// Removes the entry with this key and value from the database.
    /// Only matching entries will be deleted.
    fn remove_item<T: Table>(&mut self, table: &T, key: &T::Key, value: &T::Value);

    /// Commits the changes to the database.
    fn commit(self);

    /// Aborts the transaction and discards all changes.
    fn abort(self) {}

    /// Creates a write cursor for the given table.
    fn cursor<'txn, T: RegularTable>(&'txn self, table: &T) -> Self::WriteCursor<'txn, T>;

    /// Creates a write cursor for the given duplicate table.
    fn dup_cursor<'txn, T: DupTable>(&'txn self, table: &T) -> Self::DupWriteCursor<'txn, T>;

    /// Clears the table of all entries.
    fn clear_table<T: Table>(&mut self, table: &T);
}
