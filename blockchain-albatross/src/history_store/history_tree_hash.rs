use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Blake2bHash, Hash};
use mmr::hash::Merge;

#[derive(Clone)]
pub struct HistoryTreeHash(pub Blake2bHash);

impl HistoryTreeHash {
    pub fn to_blake2b(self) -> Blake2bHash {
        self.0
    }
}

impl Merge for HistoryTreeHash {
    fn empty(prefix: u64) -> Self {
        let message = prefix.to_be_bytes().to_vec();
        HistoryTreeHash(message.hash())
    }

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
