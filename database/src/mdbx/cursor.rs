use std::{borrow::Cow, cmp::Ordering, marker::PhantomData};

use libmdbx::{TransactionKind, WriteFlags, RO, RW};
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes};

use super::{IntoIter, IntoIterProxy};
use crate::traits::{
    DupReadCursor, DupSubKey, DupTable, DupTableValue, DupWriteCursor, ReadCursor, Row, Table,
    WriteCursor,
};

type DbKvPair<'a> = (Cow<'a, [u8]>, Cow<'a, [u8]>);

/// A cursor for navigating the entries within a table.
/// Wraps the libmdbx cursor so that we only expose our own methods.
pub struct MdbxCursor<'txn, K: TransactionKind, T: Table> {
    cursor: libmdbx::Cursor<'txn, K>,
    _table: PhantomData<T>,
}
/// Instantiation of the `MdbxCursor` for read transactions.
pub type MdbxReadCursor<'txn, T> = MdbxCursor<'txn, RO, T>;
/// Instantiation of the `MdbxCursor` for write transactions.
pub type MdbxWriteCursor<'txn, T> = MdbxCursor<'txn, RW, T>;

impl<'txn, Kind: TransactionKind, T: Table> MdbxCursor<'txn, Kind, T> {
    pub(crate) fn new(cursor: libmdbx::Cursor<'txn, Kind>) -> Self {
        MdbxCursor {
            cursor,
            _table: PhantomData,
        }
    }
}

impl<'txn, Kind: TransactionKind, T: DupTable> MdbxCursor<'txn, Kind, T>
where
    T::Value: DupTableValue,
{
    fn encode_subkey(subkey: &DupSubKey<T>) -> Cow<[u8]> {
        let enc_key = subkey.as_value_bytes();
        if let Some(new_len) = T::Value::FIXED_SIZE {
            let mut owned = enc_key.into_owned();
            owned.resize(new_len, 0);
            Cow::Owned(owned)
        } else {
            enc_key
        }
    }
}

impl<'txn, Kind: TransactionKind, T: Table> ReadCursor<'txn, T> for MdbxCursor<'txn, Kind, T> {
    type IntoIter = IntoIter<'txn, Kind, T>;

    fn first(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.first().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn last(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.last().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn next(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.next().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn prev(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.prev().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn get_current(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.get_current().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn set_key(&mut self, key: &T::Key) -> Option<T::Value> {
        let key = AsDatabaseBytes::as_key_bytes(key);
        let result: Option<Cow<[u8]>> = self.cursor.set(key.as_ref()).unwrap();
        Some(FromDatabaseBytes::from_value_bytes(&result?))
    }

    fn set_lowerbound_key(&mut self, key: &T::Key) -> Option<Row<T>> {
        let key = AsDatabaseBytes::as_key_bytes(key);
        let result: Option<DbKvPair> = self.cursor.set_range(key.as_ref()).unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn into_iter_start(self) -> Self::IntoIter {
        Self::IntoIter {
            iter: self.cursor.into_iter_start(),
            _t: PhantomData,
        }
    }

    fn into_iter_from(self, key: &T::Key) -> Self::IntoIter {
        let key = AsDatabaseBytes::as_key_bytes(key);
        Self::IntoIter {
            iter: self.cursor.into_iter_from(key.as_ref()),
            _t: PhantomData,
        }
    }
}

impl<'txn, Kind: TransactionKind, T: DupTable> DupReadCursor<'txn, T>
    for MdbxCursor<'txn, Kind, T>
{
    fn first_duplicate(&mut self) -> Option<T::Value> {
        let result: Option<Cow<[u8]>> = self.cursor.first_dup().unwrap();
        Some(FromDatabaseBytes::from_value_bytes(&result?))
    }

    fn last_duplicate(&mut self) -> Option<T::Value> {
        let result: Option<Cow<[u8]>> = self.cursor.last_dup().unwrap();
        Some(FromDatabaseBytes::from_value_bytes(&result?))
    }

    fn next_duplicate(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.next_dup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn next_no_duplicate(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.next_nodup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn prev_duplicate(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.prev_dup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn prev_no_duplicate(&mut self) -> Option<Row<T>> {
        let result: Option<DbKvPair> = self.cursor.prev_nodup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn set_subkey(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<T::Value>
    where
        T::Value: DupTableValue,
    {
        let value = self.set_lowerbound_subkey(key, subkey)?;
        if value.subkey().cmp(subkey) == Ordering::Equal {
            Some(value)
        } else {
            None
        }
    }

    fn set_lowerbound_both(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<Row<T>>
    where
        T::Value: DupTableValue,
    {
        let key = AsDatabaseBytes::as_key_bytes(key);
        let data = Self::encode_subkey(subkey);
        let result: Option<(bool, Cow<[u8]>, Cow<[u8]>)> = self
            .cursor
            .set_lowerbound(key.as_ref(), Some(data.as_ref()))
            .unwrap();
        let (_exact_match, key, value) = result?;
        Some((
            FromDatabaseBytes::from_key_bytes(&key),
            FromDatabaseBytes::from_value_bytes(&value),
        ))
    }

    fn set_lowerbound_subkey(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<T::Value>
    where
        T::Value: DupTableValue,
    {
        let key = AsDatabaseBytes::as_key_bytes(key);
        let data = Self::encode_subkey(subkey);
        let result: Option<Cow<[u8]>> = self
            .cursor
            .get_both_range(key.as_ref(), data.as_ref())
            .unwrap();
        let value = result?;
        Some(FromDatabaseBytes::from_value_bytes(&value))
    }

    fn count_duplicates(&mut self) -> usize {
        let result: Option<DbKvPair> = self.cursor.get_current().unwrap();

        if let Some((key, _)) = result {
            return self.cursor.iter_dup_of::<(), ()>(&key).count();
        } else {
            0_usize
        }
    }

    fn into_iter_dup_of(self, key: &T::Key) -> Self::IntoIter {
        let key = AsDatabaseBytes::as_key_bytes(key);
        Self::IntoIter {
            iter: self.cursor.into_iter_dup_of(key.as_ref()),
            _t: PhantomData,
        }
    }
}

impl<'txn, Kind: TransactionKind, T: Table> Clone for MdbxCursor<'txn, Kind, T> {
    fn clone(&self) -> Self {
        Self {
            cursor: self.cursor.clone(),
            _table: PhantomData,
        }
    }
}

impl<'txn, T: Table> WriteCursor<'txn, T> for MdbxWriteCursor<'txn, T> {
    fn remove(&mut self) {
        self.cursor.del(WriteFlags::empty()).unwrap();
    }

    fn append(&mut self, key: &T::Key, value: &T::Value) {
        let key = AsDatabaseBytes::as_key_bytes(key);
        let value = AsDatabaseBytes::as_value_bytes(value);
        self.cursor.put(&key, &value, WriteFlags::APPEND).unwrap();
    }

    fn put(&mut self, key: &T::Key, value: &T::Value) {
        let key = AsDatabaseBytes::as_key_bytes(key);
        let value = AsDatabaseBytes::as_value_bytes(value);
        self.cursor.put(&key, &value, WriteFlags::empty()).unwrap();
    }
}

impl<'txn, T: DupTable> DupWriteCursor<'txn, T> for MdbxWriteCursor<'txn, T> {
    fn append_dup(&mut self, key: &<T>::Key, value: &<T>::Value) {
        let key = AsDatabaseBytes::as_key_bytes(key);
        let value = AsDatabaseBytes::as_value_bytes(value);
        self.cursor
            .put(&key, &value, WriteFlags::APPEND_DUP)
            .unwrap();
    }

    fn remove_all_dup(&mut self) {
        self.cursor.del(WriteFlags::ALLDUPS).unwrap();
    }
}

/// Proxy to abstract away the transaction kind.
/// This is a read-only cursor.
pub enum CursorProxy<'txn, T: Table> {
    Read(MdbxCursor<'txn, RO, T>),
    Write(MdbxCursor<'txn, RW, T>),
}

impl<'txn, T: Table> Clone for CursorProxy<'txn, T> {
    fn clone(&self) -> Self {
        match self {
            Self::Read(cursor) => Self::Read(cursor.clone()),
            Self::Write(cursor) => Self::Write(cursor.clone()),
        }
    }
}

impl<'txn, T: Table> ReadCursor<'txn, T> for CursorProxy<'txn, T> {
    type IntoIter = IntoIterProxy<'txn, T>;

    fn first(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.first(),
            Self::Write(cursor) => cursor.first(),
        }
    }

    fn last(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.last(),
            Self::Write(cursor) => cursor.last(),
        }
    }

    fn next(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.next(),
            Self::Write(cursor) => cursor.next(),
        }
    }

    fn prev(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.prev(),
            Self::Write(cursor) => cursor.prev(),
        }
    }

    fn get_current(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.get_current(),
            Self::Write(cursor) => cursor.get_current(),
        }
    }

    fn set_key(&mut self, key: &T::Key) -> Option<<T as Table>::Value> {
        match self {
            Self::Read(cursor) => cursor.set_key(key),
            Self::Write(cursor) => cursor.set_key(key),
        }
    }

    fn set_lowerbound_key(&mut self, key: &T::Key) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.set_lowerbound_key(key),
            Self::Write(cursor) => cursor.set_lowerbound_key(key),
        }
    }

    fn into_iter_start(self) -> Self::IntoIter {
        match self {
            Self::Read(cursor) => IntoIterProxy::Read(cursor.into_iter_start()),
            Self::Write(cursor) => IntoIterProxy::Write(cursor.into_iter_start()),
        }
    }

    fn into_iter_from(self, key: &T::Key) -> Self::IntoIter {
        match self {
            Self::Read(cursor) => IntoIterProxy::Read(cursor.into_iter_from(key)),
            Self::Write(cursor) => IntoIterProxy::Write(cursor.into_iter_from(key)),
        }
    }
}

impl<'txn, T: DupTable> DupReadCursor<'txn, T> for CursorProxy<'txn, T> {
    fn first_duplicate(&mut self) -> Option<T::Value> {
        match self {
            Self::Read(cursor) => cursor.first_duplicate(),
            Self::Write(cursor) => cursor.first_duplicate(),
        }
    }

    fn last_duplicate(&mut self) -> Option<T::Value> {
        match self {
            Self::Read(cursor) => cursor.last_duplicate(),
            Self::Write(cursor) => cursor.last_duplicate(),
        }
    }

    fn next_duplicate(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.next_duplicate(),
            Self::Write(cursor) => cursor.next_duplicate(),
        }
    }

    fn next_no_duplicate(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.next_no_duplicate(),
            Self::Write(cursor) => cursor.next_no_duplicate(),
        }
    }

    fn prev_duplicate(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.prev_duplicate(),
            Self::Write(cursor) => cursor.prev_duplicate(),
        }
    }

    fn prev_no_duplicate(&mut self) -> Option<Row<T>> {
        match self {
            Self::Read(cursor) => cursor.prev_no_duplicate(),
            Self::Write(cursor) => cursor.prev_no_duplicate(),
        }
    }

    fn set_subkey(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<T::Value>
    where
        T::Value: DupTableValue,
    {
        match self {
            Self::Read(cursor) => cursor.set_subkey(key, subkey),
            Self::Write(cursor) => cursor.set_subkey(key, subkey),
        }
    }

    fn set_lowerbound_both(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<Row<T>>
    where
        T::Value: DupTableValue,
    {
        match self {
            Self::Read(cursor) => cursor.set_lowerbound_both(key, subkey),
            Self::Write(cursor) => cursor.set_lowerbound_both(key, subkey),
        }
    }

    fn set_lowerbound_subkey(&mut self, key: &T::Key, subkey: &DupSubKey<T>) -> Option<T::Value>
    where
        T::Value: DupTableValue,
    {
        match self {
            Self::Read(cursor) => cursor.set_lowerbound_subkey(key, subkey),
            Self::Write(cursor) => cursor.set_lowerbound_subkey(key, subkey),
        }
    }

    fn count_duplicates(&mut self) -> usize {
        match self {
            Self::Read(cursor) => cursor.count_duplicates(),
            Self::Write(cursor) => cursor.count_duplicates(),
        }
    }

    fn into_iter_dup_of(self, key: &T::Key) -> Self::IntoIter {
        match self {
            Self::Read(cursor) => IntoIterProxy::Read(cursor.into_iter_dup_of(key)),
            Self::Write(cursor) => IntoIterProxy::Write(cursor.into_iter_dup_of(key)),
        }
    }
}
