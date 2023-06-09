use std::cmp::Ordering;

use nimiq_hash::{Blake2bHash, Hash, HashOutput, SerializeContent};
use nimiq_hash_derive::SerializeContent;
use nimiq_keys::{PublicKey as SchnorrPublicKey, Signature as SchnorrSignature};
use nimiq_primitives::policy::Policy;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_vrf::VrfSeed;

use crate::MicroHeader;

/// Struct representing a fork proof. A fork proof proves that a given validator created or
/// continued a fork. For this it is enough to provide two different headers, with the same block
/// number, signed by the same validator.
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent)]
pub struct ForkProof {
    /// Header number 1.
    pub header1: MicroHeader,
    /// Header number 2.
    pub header2: MicroHeader,
    /// Justification for header number 1.
    pub justification1: SchnorrSignature,
    /// Justification for header number 2.
    pub justification2: SchnorrSignature,
    /// Vrf seed of the previous block. Used to determine the slot.
    pub prev_vrf_seed: VrfSeed,
}

impl ForkProof {
    /// The size of a single fork proof. This is the maximum possible size, since the Micro header
    /// has a variable size (because of the extra data field) and here we assume that the header
    /// has the maximum size.
    pub const SIZE: usize = 2 * MicroHeader::MAX_SIZE + 2 * SchnorrSignature::SIZE + VrfSeed::SIZE;

    /// Verify the validity of a fork proof.
    pub fn verify(&self, signing_key: &SchnorrPublicKey) -> Result<(), ForkProofError> {
        // Check that the headers are not equal.
        if self.header1.hash::<Blake2bHash>() == self.header2.hash::<Blake2bHash>() {
            return Err(ForkProofError::SameHeader);
        }

        // Check that the headers have equal block numbers and seeds.
        if self.header1.block_number != self.header2.block_number
            || self.header1.seed.entropy() != self.header2.seed.entropy()
        {
            return Err(ForkProofError::SlotMismatch);
        }

        if let Err(e) = self.header1.seed.verify(&self.prev_vrf_seed, signing_key) {
            error!("ForkProof: VrfSeed failed to verify: {:?}", e);
            return Err(ForkProofError::InvalidJustification);
        }

        // Check that the justifications are valid.
        let hash1 = self.header1.hash::<Blake2bHash>();
        let hash2 = self.header2.hash::<Blake2bHash>();
        if !signing_key.verify(&self.justification1, hash1.as_slice())
            || !signing_key.verify(&self.justification2, hash2.as_slice())
        {
            return Err(ForkProofError::InvalidJustification);
        }

        Ok(())
    }

    /// Check if a fork proof is valid at a given block height. Fork proofs are only during the
    /// batch when the fork was created and during batch immediately after.
    pub fn is_valid_at(&self, block_number: u32) -> bool {
        let given_batch = Policy::batch_at(block_number);

        let proof_batch = Policy::batch_at(self.header1.block_number);

        proof_batch == given_batch || proof_batch + 1 == given_batch
    }

    /// Returns the block number of a fork proof. This assumes that the fork proof is valid.
    pub fn block_number(&self) -> u32 {
        self.header1.block_number
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

impl PartialOrd for ForkProof {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ForkProof {
    fn cmp(&self, other: &Self) -> Ordering {
        self.hash::<Blake2bHash>().cmp(&other.hash::<Blake2bHash>())
    }
}

impl std::hash::Hash for ForkProof {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // We need to sort the hashes, so that it is invariant to the internal ordering.
        let mut hashes: Vec<Blake2bHash> = vec![self.header1.hash(), self.header2.hash()];
        hashes.sort();

        std::hash::Hash::hash(hashes[0].as_bytes(), state);
        std::hash::Hash::hash(hashes[1].as_bytes(), state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForkProofError {
    SlotMismatch,
    InvalidJustification,
    SameHeader,
}
