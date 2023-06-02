use crate::{
    mdbx::{MdbxReadCursor, MdbxWriteCursor},
    traits::ReadCursor,
};

pub enum CursorProxy<'txn> {
    ReadCursor(MdbxReadCursor<'txn>),
    WriteCursor(MdbxWriteCursor<'txn>),
}

impl<'txn> ReadCursor<'txn> for CursorProxy<'txn> {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.first(),
            CursorProxy::WriteCursor(cursor) => cursor.first(),
        }
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.first_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.first_duplicate(),
        }
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.last(),
            CursorProxy::WriteCursor(cursor) => cursor.last(),
        }
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.last_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.last_duplicate(),
        }
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.get_current(),
            CursorProxy::WriteCursor(cursor) => cursor.get_current(),
        }
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.next(),
            CursorProxy::WriteCursor(cursor) => cursor.next(),
        }
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.next_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.next_duplicate(),
        }
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.next_no_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.next_no_duplicate(),
        }
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.prev(),
            CursorProxy::WriteCursor(cursor) => cursor.prev(),
        }
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.prev_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.prev_duplicate(),
        }
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.prev_no_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.prev_no_duplicate(),
        }
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: nimiq_database_value::AsDatabaseBytes + ?Sized,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.seek_key(key),
            CursorProxy::WriteCursor(cursor) => cursor.seek_key(key),
        }
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: nimiq_database_value::AsDatabaseBytes + nimiq_database_value::FromDatabaseValue,
        V: nimiq_database_value::FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.seek_range_key(key),
            CursorProxy::WriteCursor(cursor) => cursor.seek_range_key(key),
        }
    }

    fn count_duplicates(&mut self) -> usize {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.count_duplicates(),
            CursorProxy::WriteCursor(cursor) => cursor.count_duplicates(),
        }
    }
}
