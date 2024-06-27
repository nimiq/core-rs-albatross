use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};

/// A cursor is used for navigating the entries within a table.
/// The read-only version cannot modify entries.
///
/// Closely follows `libmdbx`'s [cursor API](https://docs.rs/libmdbx/0.3.3/libmdbx/struct.Cursor.html).
pub trait ReadCursor<'txn>: Clone {
    type IntoIter<K, V>: Iterator<Item = (K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue;

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue;

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue;

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue;

    /// Seeks to the first entry with a key greater than or equal to the given key.
    /// For DUP tables, it also takes into account the data.
    /// The bool in the return value is set to `true` if it is an exact match.
    fn seek_range_subkey<K, V>(&mut self, key: &K, data: &V) -> Option<(bool, K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: AsDatabaseBytes + FromDatabaseValue;

    fn count_duplicates(&mut self) -> usize;

    fn into_iter_start<K, V>(self) -> Self::IntoIter<K, V>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn into_iter_dup_of<K, V>(self, key: &K) -> Self::IntoIter<K, V>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue;

    fn into_iter_from<K, V>(self, key: &K) -> Self::IntoIter<K, V>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue;
}

/// A cursor is used for navigating the entries within a table.
/// The read-write version can also delete entries.
///
/// Closely follows `libmdbx`'s [cursor API](https://docs.rs/libmdbx/0.3.3/libmdbx/struct.Cursor.html).
pub trait WriteCursor<'txn>: ReadCursor<'txn> {
    fn put<K, V>(&mut self, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    /// Appends a key/value pair to the end of the database.
    /// This operation fails if the key is less than the last key.
    fn append<K, V>(&mut self, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    /// Appends a key/value pair to the end of the database with duplicate keys.
    /// This operation fails if the key is less than the last key.
    fn append_dup<K, V>(&mut self, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    fn remove(&mut self);
}
