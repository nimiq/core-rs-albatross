use std::borrow::Cow;
use std::io;

use merkle_mountain_range::hash::Merge;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_database::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::{Blake2bHash, Hash};

/// A wrapper for the Blake2bHash. This is necessary because Rust doesn't let us implement traits
/// for structs defined in external crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryTreeHash(pub Blake2bHash);

impl HistoryTreeHash {
    pub fn unwrap(self) -> Blake2bHash {
        self.0
    }
}

impl Merge for HistoryTreeHash {
    /// Hashes just a prefix.
    fn empty(prefix: u64) -> Self {
        let message = prefix.to_be_bytes().to_vec();
        HistoryTreeHash(message.hash())
    }

    /// Hashes a prefix and two History Tree Hashes together.
    fn merge(&self, other: &Self, prefix: u64) -> Self {
        let mut message = prefix.to_be_bytes().to_vec();
        message.append(&mut self.0.serialize_to_vec());
        message.append(&mut other.0.serialize_to_vec());
        HistoryTreeHash(message.hash())
    }
}

impl Serialize for HistoryTreeHash {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        Serialize::serialize(&self.0, writer)
    }

    fn serialized_size(&self) -> usize {
        Serialize::serialized_size(&self.0)
    }
}

impl Deserialize for HistoryTreeHash {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let hash: Blake2bHash = Deserialize::deserialize(reader)?;
        Ok(HistoryTreeHash(hash))
    }
}

impl AsDatabaseBytes for HistoryTreeHash {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        self.0.as_database_bytes()
    }
}

impl FromDatabaseValue for HistoryTreeHash {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(HistoryTreeHash(bytes.into()))
    }
}
