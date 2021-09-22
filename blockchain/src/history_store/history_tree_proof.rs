use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_hash::Blake2bHash;
use nimiq_mmr::mmr::proof::Proof;

use crate::history_store::ExtendedTransaction;

/// Struct containing a vector of extended transactions together with a Merkle proof for them. It
/// allows one to prove/verify that specific transactions are part of the History Tree.
pub struct HistoryTreeProof {
    pub(crate) proof: Proof<Blake2bHash>,
    pub(crate) positions: Vec<usize>,
    pub history: Vec<ExtendedTransaction>,
}

impl HistoryTreeProof {
    /// Verifies the Merkle proof. It will return None if the verification encounters an error.
    pub fn verify(&self, expected_root: Blake2bHash) -> Option<bool> {
        assert_eq!(self.history.len(), self.positions.len());

        let mut zipped = vec![];

        for i in 0..self.positions.len() {
            zipped.push((self.positions[i], self.history[i].clone()));
        }

        self.proof.verify(&expected_root, &zipped).ok()
    }
}

impl Serialize for HistoryTreeProof {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = Serialize::serialize(&(self.proof.mmr_size as u64), writer)?;
        size += SerializeWithLength::serialize::<u32, _>(&self.proof.nodes, writer)?;
        size += SerializeWithLength::serialize::<u16, _>(
            &self
                .positions
                .iter()
                .map(|x| *x as u64)
                .collect::<Vec<u64>>(),
            writer,
        )?;
        size += SerializeWithLength::serialize::<u16, _>(&self.history, writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = Serialize::serialized_size(&(self.proof.mmr_size as u64));
        size += SerializeWithLength::serialized_size::<u32>(&self.proof.nodes);
        size += SerializeWithLength::serialized_size::<u16>(
            &self
                .positions
                .iter()
                .map(|x| *x as u64)
                .collect::<Vec<u64>>(),
        );
        size += SerializeWithLength::serialized_size::<u16>(&self.history);
        size
    }
}

impl Deserialize for HistoryTreeProof {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mmr_size: u64 = Deserialize::deserialize(reader)?;

        let proof = Proof {
            mmr_size: mmr_size as usize,
            nodes: DeserializeWithLength::deserialize::<u32, _>(reader)?,
        };

        let positions: Vec<u64> = DeserializeWithLength::deserialize::<u16, _>(reader)?;

        Ok(HistoryTreeProof {
            proof,
            positions: positions.iter().map(|x| *x as usize).collect(),
            history: DeserializeWithLength::deserialize::<u16, _>(reader)?,
        })
    }
}
