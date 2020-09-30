use crate::history_store::ExtendedTransaction;
use crate::history_store::HistoryTreeHash;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use database::{FromDatabaseValue, IntoDatabaseValue};
use mmr::error::Error;
use mmr::hash::Merge;
use mmr::mmr::MerkleMountainRange;
use mmr::store::memory::MemoryStore;
use std::io;

/// A wrapper for the MerkleMountainRange. This is necessary because Rust doesn't let us implement
/// traits for structs defined in external crates.
pub struct HistoryTree(pub MerkleMountainRange<HistoryTreeHash, MemoryStore<HistoryTreeHash>>);

impl HistoryTree {
    /// Creates a new History tree from a given Memory store.
    pub fn new(store: MemoryStore<HistoryTreeHash>) -> Self {
        HistoryTree(MerkleMountainRange::new(store))
    }

    /// Creates an empty History tree.
    pub fn empty() -> Self {
        HistoryTree::new(MemoryStore::new())
    }

    /// Returns a element of the tree by its index.
    pub fn get(&self, index: usize) -> Option<HistoryTreeHash> {
        self.0.get(index)
    }

    /// Returns the number of elements in the tree.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns a leaf of the tree by its leaf index.
    pub fn get_leaf(&self, leaf_index: usize) -> Option<HistoryTreeHash> {
        self.0.get_leaf(leaf_index)
    }

    /// Returns the number of leaf hashes in the tree.
    pub fn num_leaves(&self) -> usize {
        self.0.num_leaves()
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

        // Serialize the length of the tree.
        let length = self.len() as u64;

        // Serialize each node of the tree.
        size += Serialize::serialize(&length, writer)?;
        for i in 0..self.len() {
            size += Serialize::serialize(&self.get(i).unwrap(), writer)?;
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        // The serialized size of the MMR is simply the number of nodes times the size of each
        // serialized node.
        &self.len() * Serialize::serialized_size(&HistoryTreeHash::empty(0))
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
