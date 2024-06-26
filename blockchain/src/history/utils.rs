use std::{borrow::Cow, convert::TryInto, io};

use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::{Blake2bHash, HashOutput};

/// A wrapper for an EpochBasedIndex and a Blake2bHash. We use it to for two different functions:
/// 1) store transaction hashes in order
/// The wrapper is necessary because Rust doesn't let us implement traits for structs defined in external crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedHash {
    pub index: EpochBasedIndex,
    pub hash: Blake2bHash,
}

impl AsDatabaseBytes for OrderedHash {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let bytes = [&self.index.as_database_bytes(), self.hash.as_bytes()].concat();
        Cow::Owned(bytes)
    }
}

impl FromDatabaseValue for OrderedHash {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(OrderedHash {
            index: EpochBasedIndex::copy_from_database(&bytes[..8])?,
            hash: bytes[8..].into(),
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
