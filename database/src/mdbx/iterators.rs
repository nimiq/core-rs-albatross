use std::{borrow::Cow, marker::PhantomData};

use libmdbx::{TransactionKind, RO, RW};
use nimiq_database_value::FromDatabaseBytes;

use crate::traits::{Row, Table};

/// Iterates over database entries (key, value pairs).
/// Can be instantiated for both read and write transactions.
pub struct IntoIter<'txn, Kind: TransactionKind, T: Table> {
    pub(super) iter:
        <libmdbx::IterDup<'txn, 'txn, Kind, Cow<'txn, [u8]>, Cow<'txn, [u8]>> as Iterator>::Item,
    pub(super) _t: PhantomData<T>,
}

impl<'txn, Kind: TransactionKind, T: Table> Iterator for IntoIter<'txn, Kind, T> {
    type Item = Row<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|item| {
            let (key, value) = item.unwrap();
            (
                FromDatabaseBytes::from_key_bytes(&key),
                FromDatabaseBytes::from_value_bytes(&value),
            )
        })
    }
}

/// Proxy that abstracts away the transaction kind.
pub enum IntoIterProxy<'txn, T: Table> {
    Read(IntoIter<'txn, RO, T>),
    Write(IntoIter<'txn, RW, T>),
}

impl<'txn, T: Table> Iterator for IntoIterProxy<'txn, T> {
    type Item = Row<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIterProxy::Read(iter) => iter.next(),
            IntoIterProxy::Write(iter) => iter.next(),
        }
    }
}
