use super::{DupSubKey, DupTable, DupTableValue, Row, Table};

/// A cursor is used for navigating the entries within a table.
/// The read-only version cannot modify entries.
///
/// Closely follows `libmdbx`'s [cursor API](https://docs.rs/libmdbx/latest/libmdbx/struct.Cursor.html).
pub trait ReadCursor<'txn, T: Table>: Clone {
    type IntoIter: Iterator<Item = Row<T>>;

    /// Positions the cursor at the first entry in the table.
    fn first(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the last entry in the table.
    fn last(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the next entry in the table.
    /// For a `DupTable`, this can be either the next duplicate for the current key
    /// or the first duplicate for the next key.
    fn next(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the previous entry in the table.
    /// For a `DupTable`, this can be either the next duplicate for the current key
    /// or the first duplicate for the next key.
    fn prev(&mut self) -> Option<Row<T>>;

    /// Returns the current entry at the cursor position.
    fn get_current(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the entry that has a key == `key`.
    /// Previously `seek_key`.
    fn set_key(&mut self, key: &T::Key) -> Option<T::Value>;

    /// Positions the cursor at the first entry that has a key >= `key`.
    /// Previously `seek_range_key`.
    fn set_lowerbound_key(&mut self, key: &T::Key) -> Option<Row<T>>;

    /// Iterates over all entries in the table.
    fn into_iter_start(self) -> Self::IntoIter;

    /// Iterates over all entries in the table starting from a given key.
    fn into_iter_from(self, key: &T::Key) -> Self::IntoIter;
}

pub trait DupReadCursor<'txn, T: DupTable>: ReadCursor<'txn, T> {
    /// Positions the cursor at the first duplicate value for the current key.
    fn first_duplicate(&mut self) -> Option<T::Value>;

    /// Positions the cursor at the last duplicate value for the current key.
    fn last_duplicate(&mut self) -> Option<T::Value>;

    /// Positions the cursor at the next duplicate value for the current key.
    /// This does not jump keys.
    fn next_duplicate(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the first duplicate of the next key.
    fn next_no_duplicate(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the previous duplicate value for the current key.
    fn prev_duplicate(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the last duplicate of the previous key.
    fn prev_no_duplicate(&mut self) -> Option<Row<T>>;

    /// Positions the cursor at the entry with a given key and subkey.
    fn set_subkey(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<T::Value>
    where
        T::Value: DupTableValue;

    /// Positions the cursor at the first duplicate entry that has a key >= `key` && subkey >= `subkey`.
    fn set_lowerbound_both(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<Row<T>>
    where
        T::Value: DupTableValue;

    /// Positions the cursor at the first duplicate entry that has a key == `key` && subkey >= `subkey`.
    fn set_lowerbound_subkey(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<T::Value>
    where
        T::Value: DupTableValue;

    /// Counts the number of duplicates (linear operation!).
    fn count_duplicates(&mut self) -> usize;

    /// Iterates over all duplicates of the given key.
    fn into_iter_dup_of(self, key: &T::Key) -> Self::IntoIter;
}

/// A cursor is used for navigating the entries within a table.
/// The read-write version can also delete entries.
///
/// Closely follows `libmdbx`'s [cursor API](https://docs.rs/libmdbx/0.3.3/libmdbx/struct.Cursor.html).
pub trait WriteCursor<'txn, T: Table>: ReadCursor<'txn, T> {
    /// Puts a new entry into the database.
    /// The cursor will be positioned on the new entry.
    fn put(&mut self, key: &T::Key, value: &T::Value);

    /// Appends a key/value pair to the end of the database.
    /// This operation fails if the key is less than the last key.
    fn append(&mut self, key: &T::Key, value: &T::Value);

    /// Removes the current entry from the database.
    /// For a `DupTable`, this removes the current duplicate value.
    fn remove(&mut self);
}

pub trait DupWriteCursor<'txn, T: DupTable>: WriteCursor<'txn, T> + DupReadCursor<'txn, T> {
    /// Appends a key/value pair to the end of the database.
    /// This operation fails if the key is less than the last key.
    fn append_dup(&mut self, key: &T::Key, value: &T::Value);

    /// Removes all duplicate values for the current key.
    fn remove_all_dup(&mut self);
}
