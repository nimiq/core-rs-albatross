use nimiq_hash::Blake2bHash;
use nimiq_mmr::{
    hash::{Hash, Merge},
    mmr::proof::Proof,
};
use nimiq_serde::{Deserialize, Serialize};

use crate::historic_transaction::HistoricTransaction;

/// Struct containing a vector of historic transactions together with a Merkle proof for them. It
/// allows one to prove/verify that specific transactions are part of the History Tree.
#[derive(Serialize, Deserialize)]
pub struct HistoryTreeProof<H = Blake2bHash> {
    pub proof: Proof<H>,
    pub positions: Vec<usize>,
    pub history: Vec<HistoricTransaction>,
}

impl<H> HistoryTreeProof<H>
where
    H: Merge + Clone + Eq,
    HistoricTransaction: Hash<H>,
{
    /// Verifies the Merkle proof. It will return None if the verification encounters an error.
    pub fn verify(&self, expected_root: H) -> Option<bool> {
        assert_eq!(self.history.len(), self.positions.len());
        let zipped: Vec<_> = self
            .positions
            .iter()
            .copied()
            .zip(self.history.iter())
            .collect();
        let result = self.proof.verify(&expected_root, &zipped);
        match result {
            Ok(res) => Some(res),
            Err(error) => {
                log::error!("{:?}", error);
                None
            }
        }
    }
}
