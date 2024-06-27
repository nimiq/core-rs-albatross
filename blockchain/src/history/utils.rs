use std::{borrow::Cow, convert::TryInto, io};

use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::Blake2bHash;
use nimiq_transaction::historic_transaction::HistoricTransaction;

pub type OrderedHash = IndexedValue<EpochBasedIndex, Blake2bHash>;
pub type IndexedTransaction = IndexedValue<u32, HistoricTransaction>;
pub type IndexedHash = IndexedValue<u32, Blake2bHash>;

pub trait SubIndex: Sized {
    const INDEX_LENGTH: usize;
    fn as_be_bytes(&self) -> Cow<[u8]>;
    fn from_be_bytes(bytes: &[u8]) -> io::Result<Self>;
}

impl SubIndex for u32 {
    const INDEX_LENGTH: usize = 4;

    fn as_be_bytes(&self) -> Cow<[u8]> {
        // It is important to use to_be_bytes here for lexical ordering.
        Cow::Owned(self.to_be_bytes().into())
    }

    fn from_be_bytes(bytes: &[u8]) -> io::Result<Self> {
        // It is important to use to_be_bytes here for lexical ordering.
        Ok(u32::from_be_bytes(
            bytes.try_into().map_err(|e| io::Error::other(e))?,
        ))
    }
}

impl SubIndex for EpochBasedIndex {
    const INDEX_LENGTH: usize = 8;

    fn as_be_bytes(&self) -> Cow<[u8]> {
        self.as_database_bytes()
    }

    fn from_be_bytes(bytes: &[u8]) -> io::Result<Self> {
        FromDatabaseValue::copy_from_database(bytes)
    }
}

/// A transparent wrapper around a value to do fast lookups with `seek_range_subkey`.
/// It serializes to an empty slice, which is the lower bound.
/// When deserializing, it will can a different value.
pub struct Lookup<V: FromDatabaseValue> {
    value: Option<V>,
}

impl<V: FromDatabaseValue> Lookup<V> {
    /// Only way to initialise a Lookup.
    pub fn empty() -> Self {
        Self { value: None }
    }

    /// Should only be called after reading from the database.
    pub fn value(self) -> V {
        self.value.unwrap()
    }
}

impl<V: FromDatabaseValue> AsDatabaseBytes for Lookup<V> {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&[])
    }
}

impl<V: FromDatabaseValue> FromDatabaseValue for Lookup<V> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> {
        Ok(Lookup {
            value: Some(V::copy_from_database(bytes)?),
        })
    }
}

/// A wrapper for an EpochBasedIndex and a Blake2bHash. We use it to for two different functions:
/// 1) store transaction hashes in order
/// The wrapper is necessary because Rust doesn't let us implement traits for structs defined in external crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedValue<I: SubIndex, V: AsDatabaseBytes + FromDatabaseValue> {
    pub index: I,
    pub value: V,
}

impl<I: SubIndex, V: AsDatabaseBytes + FromDatabaseValue> IndexedValue<I, V> {
    pub fn empty(index: I) -> IndexedValue<I, Lookup<V>> {
        IndexedValue {
            index,
            value: Lookup::empty(),
        }
    }
}

impl<I: SubIndex, V: AsDatabaseBytes + FromDatabaseValue> AsDatabaseBytes for IndexedValue<I, V> {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let bytes = [
            &self.index.as_be_bytes()[..],
            &self.value.as_database_bytes()[..],
        ]
        .concat();
        Cow::Owned(bytes)
    }
}

impl<I: SubIndex, V: AsDatabaseBytes + FromDatabaseValue> FromDatabaseValue for IndexedValue<I, V> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(IndexedValue {
            index: I::from_be_bytes(&bytes[..I::INDEX_LENGTH])?,
            value: V::copy_from_database(&bytes[I::INDEX_LENGTH..])?,
        })
    }
}

/// A wrapper for an u32 and a u32.
/// We use it to store the epoch number and the (leaf) index of a transaction in the epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpochBasedIndex {
    pub epoch_number: u32,
    pub index: u32,
}

impl EpochBasedIndex {
    pub fn new(epoch_number: u32, index: u32) -> Self {
        Self {
            epoch_number,
            index,
        }
    }
}

impl AsDatabaseBytes for EpochBasedIndex {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let bytes = [
            &self.epoch_number.to_be_bytes()[..],
            &self.index.to_be_bytes()[..],
        ]
        .concat();
        Cow::Owned(bytes)
    }
}

impl FromDatabaseValue for EpochBasedIndex {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(EpochBasedIndex {
            epoch_number: u32::from_be_bytes(bytes[..4].try_into().unwrap()),
            index: u32::from_be_bytes(bytes[4..].try_into().unwrap()),
        })
    }
}
