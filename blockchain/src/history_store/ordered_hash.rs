use std::borrow::Cow;
use std::convert::TryInto;
use std::io;

use nimiq_database::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::{Blake2bHash, HashOutput};

/// A wrapper for an u32 and a Blake2bHash. We use it to for two different functions:
/// 1) store the hash and index of leaf nodes
/// 2) store transaction hashes in order
/// The wrapper is necessary because Rust doesn't let us implement traits for structs defined in external crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedHash {
    pub index: u32,
    pub hash: Blake2bHash,
}

impl AsDatabaseBytes for OrderedHash {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let bytes = [&self.index.to_be_bytes(), self.hash.as_bytes()].concat();
        Cow::Owned(bytes)
    }
}

impl FromDatabaseValue for OrderedHash {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(OrderedHash {
            index: u32::from_be_bytes(bytes[..4].try_into().unwrap()),
            hash: bytes[4..].into(),
        })
    }
}
