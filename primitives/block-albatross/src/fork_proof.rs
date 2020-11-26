use std::cmp::Ordering;
use std::io;

use beserial::{Deserialize, Serialize};
use hash::{Blake2bHash, Hash, HashOutput, SerializeContent};
use nimiq_bls::{CompressedSignature, PublicKey};
use primitives::policy;

use crate::MicroHeader;

/// Struct representing a fork proof. A fork proof proves that a given validator created or
/// continued a fork. For this it is enough to provide two different headers, with the same block
/// number and view number, signed by the same validator.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForkProof {
    /// Header number 1.
    pub header1: MicroHeader,
    /// Header number 2.
    pub header2: MicroHeader,
    /// Justification for header number 1.
    pub justification1: CompressedSignature,
    /// Justification for header number 2.
    pub justification2: CompressedSignature,
}

impl ForkProof {
    /// The size of a single fork proof. This is the maximum possible size, since the Micro header
    /// has a variable size (because of the extra data field) and here we assume that the header
    /// has the maximum size.
    pub const SIZE: usize = 2 * MicroHeader::SIZE + 2 * CompressedSignature::SIZE;

    /// Verify the validity of a fork proof.
    pub fn verify(&self, public_key: &PublicKey) -> Result<(), ForkProofError> {
        // Check that the headers are not equal.
        if self.header1.hash::<Blake2bHash>() == self.header2.hash::<Blake2bHash>() {
            return Err(ForkProofError::SameHeader);
        }

        // Check that the headers have equal block numbers and view numbers.
        if self.header1.block_number != self.header2.block_number || self.header1.view_number != self.header2.view_number {
            return Err(ForkProofError::SlotMismatch);
        }

        // Check that the justifications are valid.
        let justification1 = self.justification1.uncompress().map_err(|_| ForkProofError::InvalidJustification)?;

        let justification2 = self.justification2.uncompress().map_err(|_| ForkProofError::InvalidJustification)?;

        if !public_key.verify(&self.header1, &justification1) || !public_key.verify(&self.header2, &justification2) {
            return Err(ForkProofError::InvalidJustification);
        }

        Ok(())
    }

    /// Check if a fork proof is valid at a given block height. Fork proofs are only during the
    /// batch when the fork was created and during batch immediately after.
    pub fn is_valid_at(&self, block_number: u32) -> bool {
        let given_batch = policy::batch_at(block_number);

        let proof_batch = policy::batch_at(self.header1.block_number);

        proof_batch == given_batch || proof_batch + 1 == given_batch
    }

    /// Returns the block number of a fork proof. This assumes that the fork proof is valid.
    pub fn block_number(&self) -> u32 {
        self.header1.block_number
    }

    /// Returns the view number of a fork proof. This assumes that the fork proof is valid.
    pub fn view_number(&self) -> u32 {
        self.header1.view_number
    }
}

impl PartialEq for ForkProof {
    fn eq(&self, other: &ForkProof) -> bool {
        // Equality is invariant to ordering.
        if self.header1 == other.header1 {
            return self.header2 == other.header2 && self.justification1 == other.justification1 && self.justification2 == other.justification2;
        }

        if self.header1 == other.header2 {
            return self.header2 == other.header1 && self.justification1 == other.justification2 && self.justification2 == other.justification1;
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

impl SerializeContent for ForkProof {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for ForkProof {}

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
