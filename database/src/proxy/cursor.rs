use libmdbx::{RO, RW};
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};

use crate::{
    mdbx::{IntoIter, MdbxReadCursor, MdbxWriteCursor},
    traits::ReadCursor,
};

/// A cursor for navigating the entries within a table.
#[derive(Clone)]
pub enum CursorProxy<'txn> {
    ReadCursor(MdbxReadCursor<'txn>),
    WriteCursor(MdbxWriteCursor<'txn>),
}

impl<'txn> ReadCursor<'txn> for CursorProxy<'txn> {
    type IntoIter<K, V> = IntoIterProxy<'txn, K, V>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.first(),
            CursorProxy::WriteCursor(cursor) => cursor.first(),
        }
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.first_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.first_duplicate(),
        }
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.last(),
            CursorProxy::WriteCursor(cursor) => cursor.last(),
        }
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.last_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.last_duplicate(),
        }
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.get_current(),
            CursorProxy::WriteCursor(cursor) => cursor.get_current(),
        }
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.next(),
            CursorProxy::WriteCursor(cursor) => cursor.next(),
        }
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.next_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.next_duplicate(),
        }
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.next_no_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.next_no_duplicate(),
        }
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.prev(),
            CursorProxy::WriteCursor(cursor) => cursor.prev(),
        }
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.prev_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.prev_duplicate(),
        }
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.prev_no_duplicate(),
            CursorProxy::WriteCursor(cursor) => cursor.prev_no_duplicate(),
        }
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.seek_key(key),
            CursorProxy::WriteCursor(cursor) => cursor.seek_key(key),
        }
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.seek_range_key(key),
            CursorProxy::WriteCursor(cursor) => cursor.seek_range_key(key),
        }
    }

    fn seek_range_subkey<K, V>(&mut self, key: &K, data: &V) -> Option<(bool, K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: AsDatabaseBytes + FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.seek_range_subkey(key, data),
            CursorProxy::WriteCursor(cursor) => cursor.seek_range_subkey(key, data),
        }
    }

    fn count_duplicates(&mut self) -> usize {
        match self {
            CursorProxy::ReadCursor(cursor) => cursor.count_duplicates(),
            CursorProxy::WriteCursor(cursor) => cursor.count_duplicates(),
        }
    }

    fn into_iter_start<K, V>(self) -> Self::IntoIter<K, V>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => IntoIterProxy::ReadIter(cursor.into_iter_start()),
            CursorProxy::WriteCursor(cursor) => IntoIterProxy::WriteIter(cursor.into_iter_start()),
        }
    }

    fn into_iter_dup_of<K, V>(self, key: &K) -> Self::IntoIter<K, V>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => {
                IntoIterProxy::ReadIter(cursor.into_iter_dup_of(key))
            }
            CursorProxy::WriteCursor(cursor) => {
                IntoIterProxy::WriteIter(cursor.into_iter_dup_of(key))
            }
        }
    }

    fn into_iter_from<K, V>(self, key: &K) -> Self::IntoIter<K, V>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            CursorProxy::ReadCursor(cursor) => IntoIterProxy::ReadIter(cursor.into_iter_from(key)),
            CursorProxy::WriteCursor(cursor) => {
                IntoIterProxy::WriteIter(cursor.into_iter_from(key))
            }
        }
    }
}

/// Iterates over database entries (key, value pairs).
pub enum IntoIterProxy<'txn, K: FromDatabaseValue, V: FromDatabaseValue> {
    ReadIter(IntoIter<'txn, RO, K, V>),
    WriteIter(IntoIter<'txn, RW, K, V>),
}

impl<'txn, K: FromDatabaseValue, V: FromDatabaseValue> Iterator for IntoIterProxy<'txn, K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIterProxy::ReadIter(iter) => iter.next(),
            IntoIterProxy::WriteIter(iter) => iter.next(),
        }
    }
}
