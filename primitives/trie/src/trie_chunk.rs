use std::fmt::{self, Display};

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};

use crate::{key_nibbles::KeyNibbles, trie_proof::TrieProof};

/// The positive outcomes when committing a chunk.  
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrieChunkPushResult {
    /// The chunk was successfully applied.
    Applied,
    /// Reflects the case when the start key does not match.
    Ignored,
}

/// A helper structure for holding a trie chunk and the corresponding start key.
#[derive(Debug, Clone)]
pub struct TrieChunkWithStart {
    pub chunk: TrieChunk,
    pub start_key: KeyNibbles,
}

/// Common data structure for holding chunk items and proof.
#[derive(Debug, Clone)]
pub struct TrieChunk {
    pub keys_end: Option<KeyNibbles>,
    pub items: Vec<(KeyNibbles, Vec<u8>)>,
    pub proof: TrieProof,
}

impl Display for TrieChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TrieChunk {{ keys_end: {:?}, items: #{}, proof: .. }}",
            self.keys_end,
            self.items.len()
        )
    }
}

impl TrieChunk {
    pub fn new(
        keys_end: Option<KeyNibbles>,
        items: Vec<(KeyNibbles, Vec<u8>)>,
        proof: TrieProof,
    ) -> Self {
        Self {
            keys_end,
            items,
            proof,
        }
    }
}

impl Serialize for TrieChunk {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.keys_end.serialize(writer)?;

        size += Serialize::serialize(&(self.items.len() as u32), writer)?;
        for item in self.items.iter() {
            size += item.0.serialize(writer)?;
            size += SerializeWithLength::serialize::<u16, _>(&item.1, writer)?;
        }

        size += self.proof.serialize(writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.keys_end.serialized_size();

        size += Serialize::serialized_size(&(self.items.len() as u32));
        for item in self.items.iter() {
            size += item.0.serialized_size();
            size += SerializeWithLength::serialized_size::<u16>(&item.1);
        }

        size += self.proof.serialized_size();
        size
    }
}

impl Deserialize for TrieChunk {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let keys_end = Deserialize::deserialize(reader)?;

        let num_items: u32 = Deserialize::deserialize(reader)?;
        let mut items = Vec::with_capacity(num_items as usize);
        for _ in 0..num_items {
            let key = Deserialize::deserialize(reader)?;
            let data = DeserializeWithLength::deserialize::<u16, _>(reader)?;
            items.push((key, data));
        }

        let proof = Deserialize::deserialize(reader)?;

        Ok(TrieChunk {
            keys_end,
            items,
            proof,
        })
    }
}
