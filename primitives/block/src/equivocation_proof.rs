use std::{cmp::Ordering, hash::Hasher, mem};

use nimiq_hash::{Blake2bHash, Hash, HashOutput};
use nimiq_hash_derive::SerializeContent;
use nimiq_keys::{PublicKey as SchnorrPublicKey, Signature as SchnorrSignature};
use nimiq_primitives::policy::Policy;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_vrf::VrfSeed;

use crate::MicroHeader;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub enum EquivocationProof {
    Fork(ForkProof),
}

impl EquivocationProof {
    /// The size of a single equivocation proof. This is the maximum possible size.
    pub const SIZE: usize = ForkProof::SIZE + 1;

    pub fn is_valid_at(&self, block_number: u32) -> bool {
        use self::EquivocationProof::*;
        match self {
            Fork(proof) => proof.is_valid_at(block_number),
        }
    }

    /// Returns the key by which equivocation proofs are supposed to be sorted.
    pub fn sort_key(&self) -> Blake2bHash {
        self.hash()
    }
}

impl From<ForkProof> for EquivocationProof {
    fn from(proof: ForkProof) -> EquivocationProof {
        EquivocationProof::Fork(proof)
    }
}

/// Struct representing a fork proof. A fork proof proves that a given validator created or
/// continued a fork. For this it is enough to provide two different headers, with the same block
/// number, signed by the same validator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializeContent)]
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
    /// Make sure everyone has to go through the constructor.
    dont_construct_me: (),
}

impl ForkProof {
    /// The size of a single fork proof. This is the maximum possible size, since the Micro header
    /// has a variable size (because of the extra data field) and here we assume that the header
    /// has the maximum size.
    pub const SIZE: usize = 2 * MicroHeader::MAX_SIZE + 2 * SchnorrSignature::SIZE + VrfSeed::SIZE;

    pub fn new(
        mut header1: MicroHeader,
        mut justification1: SchnorrSignature,
        mut header2: MicroHeader,
        mut justification2: SchnorrSignature,
        prev_vrf_seed: VrfSeed,
    ) -> ForkProof {
        let hash1: Blake2bHash = header1.hash();
        let hash2: Blake2bHash = header2.hash();
        if hash1 > hash2 {
            mem::swap(&mut header1, &mut header2);
            mem::swap(&mut justification1, &mut justification2);
        }
        ForkProof {
            header1,
            header2,
            justification1,
            justification2,
            prev_vrf_seed,
            dont_construct_me: (),
        }
    }

    /// Verify the validity of a fork proof.
    pub fn verify(&self, signing_key: &SchnorrPublicKey) -> Result<(), ForkProofError> {
        let hash1: Blake2bHash = self.header1.hash();
        let hash2: Blake2bHash = self.header2.hash();

        // Check that the headers are not equal and in the right order:
        match hash1.cmp(&hash2) {
            Ordering::Less => {}
            Ordering::Equal => return Err(ForkProofError::SameHeader),
            Ordering::Greater => return Err(ForkProofError::WrongOrder),
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

    /// Check if a fork proof is valid at a given block height.
    /// Fork proofs are valid only until the end of the reporting window.
    pub fn is_valid_at(&self, block_number: u32) -> bool {
        let fork_block = self.header1.block_number;

        block_number <= Policy::last_block_of_reporting_window(fork_block)
            && Policy::batch_at(block_number) >= Policy::batch_at(fork_block)
    }

    /// Returns the block number of a fork proof. This assumes that the fork proof is valid.
    pub fn block_number(&self) -> u32 {
        self.header1.block_number
    }
}

impl std::hash::Hash for ForkProof {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(self.header1.hash::<Blake2bHash>().as_bytes());
        state.write(self.header2.hash::<Blake2bHash>().as_bytes());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForkProofError {
    SlotMismatch,
    InvalidJustification,
    SameHeader,
    WrongOrder,
}
