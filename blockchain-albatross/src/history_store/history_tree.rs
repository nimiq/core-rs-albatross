use crate::history_store::extended_transaction::ExtendedTransaction;
use crate::history_store::history_tree_hash::HistoryTreeHash;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use database::{FromDatabaseValue, IntoDatabaseValue};
use mmr::error::Error;
use mmr::mmr::MerkleMountainRange;
use mmr::store::memory::MemoryStore;
use std::io;

pub struct HistoryTree(pub MerkleMountainRange<HistoryTreeHash, MemoryStore<HistoryTreeHash>>);

impl HistoryTree {
    pub fn new(store: MemoryStore<HistoryTreeHash>) -> Self {
        HistoryTree(MerkleMountainRange::new(store))
    }

    /// Returns the number of elements in the tree.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Inserts an element and returns the corresponding leaf index.
    /// The leaf index is not the internal index within the tree,
    /// but a leaf index of i means that it is the i-th leaf (starting to count at 0).
    pub fn push(&mut self, elem: &ExtendedTransaction) -> Result<usize, Error> {
        self.0.push(elem)
    }

    /// Calculates the root.
    pub fn get_root(&self) -> Result<HistoryTreeHash, Error> {
        self.0.get_root()
    }
}

impl Serialize for HistoryTree {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        let length = self.0.len() as u64;
        size += Serialize::serialize(&length, writer)?;
        for h in &self.0.store.inner {
            size += Serialize::serialize(h, writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        for h in &self.0.store.inner {
            size += Serialize::serialized_size(h);
        }
        size
    }
}

impl Deserialize for HistoryTree {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut store: Vec<HistoryTreeHash> = vec![];
        let length: u64 = Deserialize::deserialize(reader)?;
        for _i in 0..length {
            store.push(Deserialize::deserialize(reader)?);
        }
        Ok(HistoryTree::new(MemoryStore { inner: store }))
    }
}

impl IntoDatabaseValue for HistoryTree {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for HistoryTree {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
