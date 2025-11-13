use std::fmt::{self, Debug, Formatter};

use nimiq_hash::Blake2bHash;
use nimiq_mmr::{
    hash::{Hash, Merge},
    mmr::proof::RangeProof,
};
use nimiq_transaction::historic_transaction::HistoricTransaction;
use serde::{Deserialize, Serialize};

/// The chunk size used in our protocol.
/// TODO: Update number.
pub const CHUNK_SIZE: usize = 1024;

#[derive(Serialize, Deserialize)]
pub struct HistoryTreeChunk<H = Blake2bHash> {
    pub(crate) proof: RangeProof<H>,
    pub history: Vec<HistoricTransaction>,
}

impl<H> Debug for HistoryTreeChunk<H> {
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
        dbg.finish()
    }
}

impl<H> HistoryTreeChunk<H>
where
    H: Merge + Clone + Eq,
    HistoricTransaction: Hash<H>,
{
    /// Tries to verify
    pub fn verify(&self, expected_root: &H, leaf_index: usize) -> Option<bool> {
        self.proof
            .verify_with_start(expected_root, leaf_index, &self.history)
            .ok()
    }
}
