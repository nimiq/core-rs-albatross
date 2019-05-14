use beserial::{Deserialize, Serialize};
use hash::{Blake2bHash, Hash};
use nimiq_bls::bls12_381::{PublicKey, Signature};

use crate::MicroHeader;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForkProof {
    pub header1: MicroHeader,
    pub header2: MicroHeader,
    pub justification1: Signature,
    pub justification2: Signature,
}

impl ForkProof {
    pub const SIZE: usize = 2 * MicroHeader::SIZE + 2 * 48;

    pub fn pre_verify(&self) -> Option<(/* block_number */ u32, /* view_number */ u32)> {
        let block_number = self.header1.block_number;
        let view_number = self.header1.view_number;
        if block_number == self.header2.block_number && view_number == self.header2.view_number {
            Some((block_number, view_number))
        }
        else {
            None
        }
    }

    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key.verify(&self.header1, &self.justification1)
            && public_key.verify(&self.header2, &self.justification2)
    }
}

impl PartialEq for ForkProof {
    fn eq(&self, other: &ForkProof) -> bool {
        // Equality is invariant to ordering.
        if self.header1 == other.header1 {
            return self.header2 == other.header2
                && self.justification1 == other.justification1
                && self.justification2 == other.justification2;
        }

        if self.header1 == other.header2 {
            return self.header2 == other.header1
                && self.justification1 == other.justification2
                && self.justification2 == other.justification1;
        }

        false
    }
}

impl Eq for ForkProof {}

impl std::hash::Hash for ForkProof {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // We need to sort the hashes, so that it is invariant to the internal ordering.
        let mut hashes: Vec<Blake2bHash> = vec![self.header1.hash(), self.header2.hash()];
        hashes.sort();

        std::hash::Hash::hash(hashes[0].as_bytes(), state);
        std::hash::Hash::hash(hashes[1].as_bytes(), state);
    }
}
