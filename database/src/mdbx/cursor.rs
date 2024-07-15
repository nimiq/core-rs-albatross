use std::{borrow::Cow, marker::PhantomData, sync::Arc};

use libmdbx::{TransactionKind, WriteFlags, RO, RW};
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};

use super::{DbKvPair, IntoIter};
use crate::{
    metrics::{DatabaseEnvMetrics, Operation},
    traits::{ReadCursor, WriteCursor},
};

/// A cursor for navigating the entries within a table.
/// Wraps the libmdbx cursor so that we only expose our own methods.
pub struct MdbxCursor<'txn, K: TransactionKind> {
    cursor: libmdbx::Cursor<'txn, K>,
    metrics: Option<Arc<DatabaseEnvMetrics>>,
    table_name: String,
}
/// Instantiation of the `MdbxCursor` for read transactions.
pub type MdbxReadCursor<'txn> = MdbxCursor<'txn, RO>;
/// Instantiation of the `MdbxCursor` for write transactions.
pub type MdbxWriteCursor<'txn> = MdbxCursor<'txn, RW>;

impl<'txn, Kind> MdbxCursor<'txn, Kind>
where
    Kind: TransactionKind,
{
    pub(crate) fn new(
        table_name: &str,
        cursor: libmdbx::Cursor<'txn, Kind>,
        metrics: Option<Arc<DatabaseEnvMetrics>>,
    ) -> Self {
        MdbxCursor {
            table_name: table_name.to_string(),
            cursor,
            metrics,
        }
    }

    /// If `self.metrics` is `Some(...)`, record a metric with the provided operation and value
    /// size.
    ///
    /// Otherwise, just execute the closure.
    fn execute_with_operation_metric<R>(
        &mut self,
        operation: Operation,
        value_size: Option<usize>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        if let Some(metrics) = self.metrics.as_ref().cloned() {
            metrics.record_operation(&self.table_name.clone(), operation, value_size, || f(self))
        } else {
            f(self)
        }
    }
}

impl<'txn, Kind> ReadCursor<'txn> for MdbxCursor<'txn, Kind>
where
    Kind: TransactionKind,
{
    type IntoIter<K, V> = IntoIter<'txn, Kind, K, V>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.first().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        let result: Option<Cow<[u8]>> = self.cursor.first_dup().unwrap();
        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.last().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        let result: Option<Cow<[u8]>> = self.cursor.last_dup().unwrap();
        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.get_current().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.next().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.next_dup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.next_nodup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.prev().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.prev_dup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.prev_nodup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<Cow<[u8]>> = self.cursor.set(key.as_ref()).unwrap();
        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<DbKvPair> = self.cursor.set_range(key.as_ref()).unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn seek_range_subkey<K, V>(&mut self, key: &K, data: &V) -> Option<(bool, K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: AsDatabaseBytes + FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let data = AsDatabaseBytes::as_database_bytes(data);
        let result: Option<(bool, Cow<[u8]>, Cow<[u8]>)> = self
            .cursor
            .set_lowerbound(key.as_ref(), Some(data.as_ref()))
            .unwrap();
        let (exact_match, key, value) = result?;
        Some((
            exact_match,
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn count_duplicates(&mut self) -> usize {
        let result: Option<DbKvPair> = self.cursor.get_current().unwrap();

        if let Some((key, _)) = result {
            return self.cursor.iter_dup_of::<(), ()>(&key).count();
        } else {
            0_usize
        }
    }

    fn into_iter_start<K, V>(self) -> Self::IntoIter<K, V>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        Self::IntoIter {
            iter: self.cursor.into_iter_start(),
            _k: PhantomData,
            _v: PhantomData,
        }
    }

    fn into_iter_dup_of<K, V>(self, key: &K) -> Self::IntoIter<K, V>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        Self::IntoIter {
            iter: self.cursor.into_iter_dup_of(key.as_ref()),
            _k: PhantomData,
            _v: PhantomData,
        }
    }

    fn into_iter_from<K, V>(self, key: &K) -> Self::IntoIter<K, V>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        Self::IntoIter {
            iter: self.cursor.into_iter_from(key.as_ref()),
            _k: PhantomData,
            _v: PhantomData,
        }
    }
}

impl<'txn, Kind> Clone for MdbxCursor<'txn, Kind>
where
    Kind: TransactionKind,
{
    fn clone(&self) -> Self {
        Self {
            table_name: self.table_name.clone(),
            cursor: self.cursor.clone(),
            metrics: self.metrics.as_ref().cloned(),
        }
    }
}

impl<'txn> WriteCursor<'txn> for MdbxWriteCursor<'txn> {
    fn remove(&mut self) {
        self.execute_with_operation_metric(Operation::CursorDeleteCurrent, None, |cursor| {
            cursor.cursor.del(WriteFlags::empty()).unwrap();
        });
    }

    fn append<K, V>(&mut self, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);
        self.cursor.put(&key, &value, WriteFlags::APPEND).unwrap();
    }

    fn append_dup<K, V>(&mut self, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);
        self.cursor
            .put(&key, &value, WriteFlags::APPEND_DUP)
            .unwrap();
    }

    fn put<K, V>(&mut self, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);
        self.cursor.put(&key, &value, WriteFlags::empty()).unwrap();
    }
}
