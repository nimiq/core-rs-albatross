use std::cmp::Ordering;
use std::io;

use beserial::{Deserialize, Serialize};
use hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_bls::{CompressedSignature, PublicKey};
use primitives::policy;

use crate::MicroHeader;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForkProof {
    pub header1: MicroHeader,
    pub header2: MicroHeader,
    pub justification1: CompressedSignature,
    pub justification2: CompressedSignature,
}

impl ForkProof {
    pub const SIZE: usize = 2 * MicroHeader::SIZE + 2 * 48;

    pub fn verify(&self, public_key: &PublicKey) -> Result<(), ForkProofError> {
        // XXX Duplicate check
        if self.header1.block_number != self.header2.block_number
            || self.header1.view_number != self.header2.view_number
        {
            return Err(ForkProofError::SlotMismatch);
        }

        let justification1 = self
            .justification1
            .uncompress()
            .map_err(|_| ForkProofError::InvalidJustification)?;
        let justification2 = self
            .justification2
            .uncompress()
            .map_err(|_| ForkProofError::InvalidJustification)?;

        if !public_key.verify(&self.header1, &justification1)
            || !public_key.verify(&self.header2, &justification2)
        {
            return Err(ForkProofError::InvalidJustification);
        }

        Ok(())
    }

    pub fn is_valid_at(&self, block_number: u32) -> bool {
        let given_epoch = policy::epoch_at(block_number);
        let proof_epoch = policy::epoch_at(self.header1.block_number);
        self.header1.block_number == self.header2.block_number
            && self.header1.view_number == self.header2.view_number
            // XXX Should this be checked at a higher layer?
            && (proof_epoch == given_epoch || proof_epoch + 1 == given_epoch)
    }

    pub fn block_number(&self) -> u32 {
        self.header1.block_number
    }

    pub fn view_number(&self) -> u32 {
        self.header1.view_number
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
}
