use std::borrow::Cow;
use std::io;

use nimiq_database::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::{Blake2bHash, HashOutput};
use std::convert::TryInto;

/// A wrapper for the Blake2bHash and a u32. We use it to store the hash and index of a leaf node.
/// This is necessary because Rust doesn't let us implement traits for structs defined in external crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafData {
    pub hash: Blake2bHash,
    pub index: u32,
}

impl AsDatabaseBytes for LeafData {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let bytes = [self.hash.as_bytes(), &self.index.to_be_bytes()].concat();
        Cow::Owned(bytes)
    }
}

impl FromDatabaseValue for LeafData {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(LeafData {
            hash: bytes[..32].into(),
            index: u32::from_be_bytes(bytes[32..].try_into().unwrap()),
        })
    }
}
