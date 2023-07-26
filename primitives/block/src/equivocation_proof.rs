use std::{cmp::Ordering, hash::Hasher, mem};

use nimiq_bls::{AggregatePublicKey, AggregateSignature};
use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, HashOutput};
use nimiq_hash_derive::SerializeContent;
use nimiq_keys::{PublicKey as SchnorrPublicKey, Signature as SchnorrSignature};
use nimiq_primitives::{policy::Policy, slots_allocation::Validators};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_vrf::VrfSeed;

use crate::{MacroHeader, MicroHeader, TendermintIdentifier, TendermintVote};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub enum EquivocationProof {
    Fork(ForkProof),
    DoubleProposal(DoubleProposalProof),
    DoubleVote(DoubleVoteProof),
}

impl EquivocationProof {
    /// The size of a single equivocation proof. This is the maximum possible size.
    pub const SIZE: usize = ForkProof::SIZE + 1;

    /// Returns the block number of an equivocation proof. This assumes that the equivocation proof
    /// is valid.
    pub fn block_number(&self) -> u32 {
        use self::EquivocationProof::*;
        match self {
            Fork(proof) => proof.block_number(),
            DoubleProposal(proof) => proof.block_number(),
            DoubleVote(proof) => proof.block_number(),
        }
    }

    /// Check if an equivocation proof is valid at a given block height. Equivocation proofs are
    /// valid only until the end of the reporting window.
    pub fn is_valid_at(&self, block_number: u32) -> bool {
        block_number <= Policy::last_block_of_reporting_window(self.block_number())
            && Policy::batch_at(block_number) >= Policy::batch_at(self.block_number())
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
    header1: MicroHeader,
    /// Header number 2.
    header2: MicroHeader,
    /// Justification for header number 1.
    justification1: SchnorrSignature,
    /// Justification for header number 2.
    justification2: SchnorrSignature,
    /// Vrf seed of the previous block. Used to determine the slot.
    prev_vrf_seed: VrfSeed,
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
        }
    }

    pub fn header1_hash(&self) -> Blake2bHash {
        self.header1.hash()
    }
    pub fn header2_hash(&self) -> Blake2bHash {
        self.header2.hash()
    }
    pub fn block_number(&self) -> u32 {
        self.header1.block_number
    }
    pub fn prev_vrf_seed(&self) -> &VrfSeed {
        &self.prev_vrf_seed
    }

    /// Verify the validity of a fork proof.
    pub fn verify(&self, signing_key: &SchnorrPublicKey) -> Result<(), EquivocationProofError> {
        let hash1: Blake2bHash = self.header1.hash();
        let hash2: Blake2bHash = self.header2.hash();

        // Check that the headers are not equal and in the right order:
        match hash1.cmp(&hash2) {
            Ordering::Less => {}
            Ordering::Equal => return Err(EquivocationProofError::SameHeader),
            Ordering::Greater => return Err(EquivocationProofError::WrongOrder),
        }

        // Check that the headers have equal block numbers and seeds.
        if self.header1.block_number != self.header2.block_number
            || self.header1.seed.entropy() != self.header2.seed.entropy()
        {
            return Err(EquivocationProofError::SlotMismatch);
        }

        if let Err(e) = self.header1.seed.verify(&self.prev_vrf_seed, signing_key) {
            error!("ForkProof: VrfSeed failed to verify: {:?}", e);
            return Err(EquivocationProofError::InvalidJustification);
        }

        // Check that the justifications are valid.
        if !signing_key.verify(&self.justification1, hash1.as_slice())
            || !signing_key.verify(&self.justification2, hash2.as_slice())
        {
            return Err(EquivocationProofError::InvalidJustification);
        }

        Ok(())
    }
}

impl std::hash::Hash for ForkProof {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(self.header1.hash::<Blake2bHash>().as_bytes());
        state.write(self.header2.hash::<Blake2bHash>().as_bytes());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquivocationProofError {
    SlotMismatch,
    InvalidJustification,
    InvalidSlotNumber,
    SameHeader,
    WrongOrder,
}

/// Struct representing a double proposal proof. A double proposal proof proves that a given
/// validator created two macro block proposals at the same height, in the same round.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializeContent)]
pub struct DoubleProposalProof {
    header1: MacroHeader,
    header2: MacroHeader,
    justification1: SchnorrSignature,
    justification2: SchnorrSignature,
    prev_vrf_seed1: VrfSeed,
    prev_vrf_seed2: VrfSeed,
}

impl DoubleProposalProof {
    pub fn new(
        mut header1: MacroHeader,
        mut justification1: SchnorrSignature,
        mut prev_vrf_seed1: VrfSeed,
        mut header2: MacroHeader,
        mut justification2: SchnorrSignature,
        mut prev_vrf_seed2: VrfSeed,
    ) -> DoubleProposalProof {
        let hash1: Blake2bHash = header1.hash();
        let hash2: Blake2bHash = header2.hash();
        if hash1 > hash2 {
            mem::swap(&mut header1, &mut header2);
            mem::swap(&mut justification1, &mut justification2);
            mem::swap(&mut prev_vrf_seed1, &mut prev_vrf_seed2);
        }
        DoubleProposalProof {
            header1,
            header2,
            justification1,
            justification2,
            prev_vrf_seed1,
            prev_vrf_seed2,
        }
    }

    pub fn block_number(&self) -> u32 {
        self.header1.block_number
    }
    pub fn round(&self) -> u32 {
        self.header1.round
    }
    pub fn header1_hash(&self) -> Blake2bHash {
        self.header1.hash()
    }
    pub fn header2_hash(&self) -> Blake2bHash {
        self.header2.hash()
    }
    pub fn prev_vrf_seed1(&self) -> &VrfSeed {
        &self.prev_vrf_seed1
    }
    pub fn prev_vrf_seed2(&self) -> &VrfSeed {
        &self.prev_vrf_seed2
    }

    /// Verify the validity of a double proposal proof.
    pub fn verify(&self, signing_key: &SchnorrPublicKey) -> Result<(), EquivocationProofError> {
        let hash1: Blake2bHash = self.header1.hash();
        let hash2: Blake2bHash = self.header2.hash();

        // Check that the headers are not equal and in the right order:
        match hash1.cmp(&hash2) {
            Ordering::Less => {}
            Ordering::Equal => return Err(EquivocationProofError::SameHeader),
            Ordering::Greater => return Err(EquivocationProofError::WrongOrder),
        }

        if self.header1.block_number != self.header2.block_number
            || self.header1.round != self.header2.round
        {
            return Err(EquivocationProofError::SlotMismatch);
        }

        if let Err(e) = self.header1.seed.verify(&self.prev_vrf_seed1, signing_key) {
            error!("DoubleProposalProof: VrfSeed 1 failed to verify: {:?}", e);
            return Err(EquivocationProofError::InvalidJustification);
        }

        if let Err(e) = self.header2.seed.verify(&self.prev_vrf_seed2, signing_key) {
            error!("DoubleProposalProof: VrfSeed 2 failed to verify: {:?}", e);
            return Err(EquivocationProofError::InvalidJustification);
        }

        // Check that the justifications are valid.
        if !signing_key.verify(&self.justification1, hash1.as_slice())
            || !signing_key.verify(&self.justification2, hash2.as_slice())
        {
            return Err(EquivocationProofError::InvalidJustification);
        }

        Ok(())
    }
}

impl std::hash::Hash for DoubleProposalProof {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(Hash::hash::<Blake2bHash>(self).as_bytes());
    }
}

/// Struct representing a double vote proof. A double vote proof proves that a given
/// validator voted twice at same height, in the same round.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializeContent)]
pub struct DoubleVoteProof {
    tendermint_id: TendermintIdentifier,
    slot_number: u16,
    proposal_hash1: Option<Blake2sHash>,
    proposal_hash2: Option<Blake2sHash>,
    signature1: AggregateSignature,
    signature2: AggregateSignature,
    signers1: BitSet,
    signers2: BitSet,
}

impl DoubleVoteProof {
    pub fn new(
        tendermint_id: TendermintIdentifier,
        slot_number: u16,
        mut proposal_hash1: Option<Blake2sHash>,
        mut proposal_hash2: Option<Blake2sHash>,
        mut signature1: AggregateSignature,
        mut signature2: AggregateSignature,
        mut signers1: BitSet,
        mut signers2: BitSet,
    ) -> DoubleVoteProof {
        if proposal_hash1 > proposal_hash2 {
            mem::swap(&mut proposal_hash1, &mut proposal_hash2);
            mem::swap(&mut signature1, &mut signature2);
            mem::swap(&mut signers1, &mut signers2);
        }
        DoubleVoteProof {
            tendermint_id,
            slot_number,
            proposal_hash1,
            proposal_hash2,
            signature1,
            signature2,
            signers1,
            signers2,
        }
    }

    pub fn block_number(&self) -> u32 {
        self.tendermint_id.block_number
    }
    pub fn slot_number(&self) -> u16 {
        self.slot_number
    }

    /// Verify the validity of a double vote proof.
    ///
    /// The parameter `validators` must be the historic validators of that
    /// macro block production.
    pub fn verify(&self, validators: &Validators) -> Result<(), EquivocationProofError> {
        // Check that the proposals are not equal and in the right order:
        match self.proposal_hash1.cmp(&self.proposal_hash2) {
            Ordering::Less => {}
            Ordering::Equal => return Err(EquivocationProofError::SameHeader),
            Ordering::Greater => return Err(EquivocationProofError::WrongOrder),
        }

        if self.slot_number >= Policy::SLOTS {
            return Err(EquivocationProofError::InvalidSlotNumber);
        }

        // Check that the signatures actually contain the reported validator.
        if !self.signers1.contains(self.slot_number as usize) {
            // The validator did not participate in the signature.
            return Err(EquivocationProofError::SlotMismatch);
        }
        if !self.signers2.contains(self.slot_number as usize) {
            // The validator did not participate in the signature.
            return Err(EquivocationProofError::SlotMismatch);
        }

        // Calculate the messages that were actually signed by the validators.
        let message1 = TendermintVote {
            proposal_hash: self.proposal_hash1.clone(),
            id: self.tendermint_id.clone(),
        };
        let message2 = TendermintVote {
            proposal_hash: self.proposal_hash2.clone(),
            id: self.tendermint_id.clone(),
        };

        // Verify the signatures.
        {
            let mut agg_pk1 = AggregatePublicKey::new();
            for (i, pk) in validators.voting_keys().iter().enumerate() {
                if self.signers1.contains(i) {
                    agg_pk1.aggregate(pk);
                }
            }
            if !agg_pk1.verify(&message1, &self.signature1) {
                return Err(EquivocationProofError::InvalidJustification);
            }
        }
        {
            let mut agg_pk2 = AggregatePublicKey::new();
            for (i, pk) in validators.voting_keys().iter().enumerate() {
                if self.signers2.contains(i) {
                    agg_pk2.aggregate(pk);
                }
            }
            if !agg_pk2.verify(&message2, &self.signature2) {
                return Err(EquivocationProofError::InvalidJustification);
            }
        }

        Ok(())
    }
}

impl std::hash::Hash for DoubleVoteProof {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(Hash::hash::<Blake2bHash>(self).as_bytes());
    }
}
