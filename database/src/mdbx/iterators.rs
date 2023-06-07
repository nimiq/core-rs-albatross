use std::{borrow::Cow, marker::PhantomData};

use libmdbx::TransactionKind;

use nimiq_database_value::FromDatabaseValue;

/// Iterates over database entries (key, value pairs).
/// Can be instantiated for both read and write transactions.
pub struct IntoIter<'txn, Kind: TransactionKind, K: FromDatabaseValue, V: FromDatabaseValue> {
    pub(super) iter:
        <libmdbx::IterDup<'txn, 'txn, Kind, Cow<'txn, [u8]>, Cow<'txn, [u8]>> as Iterator>::Item,
    pub(super) _k: PhantomData<K>,
    pub(super) _v: PhantomData<V>,
}

impl<'txn, Kind: TransactionKind, K: FromDatabaseValue, V: FromDatabaseValue> Iterator
    for IntoIter<'txn, Kind, K, V>
{
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|item| {
            let (key, value) = item.unwrap();
            (
                FromDatabaseValue::copy_from_database(&key).unwrap(),
                FromDatabaseValue::copy_from_database(&value).unwrap(),
            )
        })
    }
}
