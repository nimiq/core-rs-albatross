use std::fmt::{self, Debug, Formatter};

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_hash::Blake2bHash;
use nimiq_mmr::mmr::proof::{Proof, RangeProof};

use crate::history_store::ExtendedTransaction;

/// The chunk size used in our protocol.
/// TODO: Update number.
pub const CHUNK_SIZE: usize = 1000;

pub struct HistoryTreeChunk {
    pub(crate) proof: RangeProof<Blake2bHash>,
    pub history: Vec<ExtendedTransaction>,
}

impl Debug for HistoryTreeChunk {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let mut dbg = f.debug_struct("HistoryTreeChunk");
        let len = self.history.len();
        dbg.field("length", &len);
        if !self.history.is_empty() {
            let first = self.history.first().unwrap();
            let last = self.history.last().unwrap();
            dbg.field("first_block", &first.block_number);
            dbg.field("last_block", &last.block_number);
        }
        dbg.field("history", &self.history);
        dbg.finish()
    }
}

impl HistoryTreeChunk {
    /// Tries to verify
    pub fn verify(&self, expected_root: Blake2bHash, leaf_index: usize) -> Option<bool> {
        // TODO: Modify MMR library so that we do not need to clone here.
        self.proof
            .verify_with_start(&expected_root, leaf_index, self.history.clone())
            .ok()
    }
}

impl Serialize for HistoryTreeChunk {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = Serialize::serialize(&self.proof.assume_previous, writer)?;
        size += Serialize::serialize(&(self.proof.proof.mmr_size as u64), writer)?;
        size += SerializeWithLength::serialize::<u32, _>(&self.proof.proof.nodes, writer)?;

        size += SerializeWithLength::serialize::<u16, _>(&self.history, writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = Serialize::serialized_size(&self.proof.assume_previous);
        size += Serialize::serialized_size(&(self.proof.proof.mmr_size as u64));
        size += SerializeWithLength::serialized_size::<u32>(&self.proof.proof.nodes);

        size += SerializeWithLength::serialized_size::<u16>(&self.history);
        size
    }
}

impl Deserialize for HistoryTreeChunk {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let assume_previous: bool = Deserialize::deserialize(reader)?;
        let mmr_size: u64 = Deserialize::deserialize(reader)?;

        let proof = RangeProof {
            proof: Proof {
                mmr_size: mmr_size as usize,
                nodes: DeserializeWithLength::deserialize::<u32, _>(reader)?,
            },
            assume_previous,
        };

        Ok(HistoryTreeChunk {
            proof,
            history: DeserializeWithLength::deserialize::<u16, _>(reader)?,
        })
    }
}
