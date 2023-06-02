use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};

pub trait ReadCursor<'txn> {
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

    fn count_duplicates(&mut self) -> usize;
}

pub trait WriteCursor<'txn>: ReadCursor<'txn> {
    fn remove(&mut self);
}
