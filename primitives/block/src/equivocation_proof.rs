use std::{cmp::Ordering, hash::Hasher, io, mem, ops::Range};

use nimiq_bls::{AggregatePublicKey, AggregateSignature};
use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash as _, HashOutput, SerializeContent};
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey, Signature as SchnorrSignature};
use nimiq_primitives::{
    policy::Policy,
    slots_allocation::{Validator, Validators},
    TendermintIdentifier, TendermintStep, TendermintVote,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    DoubleProposalLocator, DoubleVoteLocator, EquivocationLocator, ForkLocator,
};
use thiserror::Error;

use crate::{MacroHeader, MicroHeader};

/// An equivocation proof proves that a validator misbehaved.
///
/// This can come in several forms, but e.g. producing two blocks in a single slot or voting twice
/// in the same round.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EquivocationProof {
    Fork(ForkProof),
    DoubleProposal(DoubleProposalProof),
    DoubleVote(DoubleVoteProof),
}

const fn max3(a: usize, b: usize, c: usize) -> usize {
    if a > b && a > c {
        a
    } else if b > c {
        b
    } else {
        c
    }
}

fn get_validator<'a>(
    validators: &'a Validators,
    address: &Address,
) -> Result<&'a Validator, EquivocationProofError> {
    validators
        .get_validator_by_address(address)
        .ok_or(EquivocationProofError::InvalidValidatorAddress)
}

impl EquivocationProof {
    /// The size of a single equivocation proof. This is the maximum possible size.
    pub const MAX_SIZE: usize = 1 + max3(
        ForkProof::MAX_SIZE,
        DoubleProposalProof::MAX_SIZE,
        DoubleVoteProof::MAX_SIZE,
    );

    /// Locator of this proof.
    ///
    /// It is used to check that only one proof of an equivocation can be
    /// included for a given equivocation locator.
    pub fn locator(&self) -> EquivocationLocator {
        use self::EquivocationProof::*;
        match self {
            Fork(proof) => proof.locator().into(),
            DoubleProposal(proof) => proof.locator().into(),
            DoubleVote(proof) => proof.locator().into(),
        }
    }

    /// Address of the offending validator.
    pub fn validator_address(&self) -> &Address {
        use self::EquivocationProof::*;
        match self {
            Fork(proof) => proof.validator_address(),
            DoubleProposal(proof) => proof.validator_address(),
            DoubleVote(proof) => proof.validator_address(),
        }
    }

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

    /// Check if an equivocation proof contains a valid offense.
    ///
    /// The parameter `validators` must be the historic validators at the time of the offense.
    pub fn verify(&self, validators: &Validators) -> Result<(), EquivocationProofError> {
        self.verify_excluding_address(
            validators,
            get_validator(validators, self.validator_address())?,
        )
    }

    /// Check if an equivocation proof contains a valid offense, but do not check attribution to
    /// the `validator_address`.
    pub fn verify_excluding_address(
        &self,
        validators: &Validators,
        validator: &Validator,
    ) -> Result<(), EquivocationProofError> {
        use self::EquivocationProof::*;
        match self {
            Fork(proof) => proof.verify_excluding_address(&validator.signing_key),
            DoubleProposal(proof) => proof.verify_excluding_address(&validator.signing_key),
            DoubleVote(proof) => {
                proof.verify_excluding_address(validators, validator.slots.clone())
            }
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

impl From<DoubleProposalProof> for EquivocationProof {
    fn from(proof: DoubleProposalProof) -> EquivocationProof {
        EquivocationProof::DoubleProposal(proof)
    }
}

impl From<DoubleVoteProof> for EquivocationProof {
    fn from(proof: DoubleVoteProof) -> EquivocationProof {
        EquivocationProof::DoubleVote(proof)
    }
}

impl std::hash::Hash for EquivocationProof {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.locator(), state)
    }
}

impl SerializeContent for EquivocationProof {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        self.locator().serialize_content::<_, H>(writer)
    }
}

/// Struct representing a fork proof. A fork proof proves that a given validator created or
/// continued a fork. For this it is enough to provide two different headers, with the same block
/// number, signed by the same validator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForkProof {
    /// Address of the offending validator.
    validator_address: Address,
    /// Header number 1.
    header1: MicroHeader,
    /// Header number 2.
    header2: MicroHeader,
    /// Justification for header number 1.
    justification1: SchnorrSignature,
    /// Justification for header number 2.
    justification2: SchnorrSignature,
}

impl ForkProof {
    /// The size of a single fork proof. This is the maximum possible size, since the Micro header
    /// has a variable size (because of the extra data field) and here we assume that the header
    /// has the maximum size.
    pub const MAX_SIZE: usize =
        Address::SIZE + 2 * MicroHeader::MAX_SIZE + 2 * SchnorrSignature::SIZE;

    pub fn new(
        validator_address: Address,
        mut header1: MicroHeader,
        mut justification1: SchnorrSignature,
        mut header2: MicroHeader,
        mut justification2: SchnorrSignature,
    ) -> ForkProof {
        let hash1: Blake2bHash = header1.hash();
        let hash2: Blake2bHash = header2.hash();
        if hash1 > hash2 {
            mem::swap(&mut header1, &mut header2);
            mem::swap(&mut justification1, &mut justification2);
        }
        ForkProof {
            validator_address,
            header1,
            header2,
            justification1,
            justification2,
        }
    }

    /// Locator of this proof.
    ///
    /// It is used to check that only one fork proof can be included for a
    /// given block height.
    pub fn locator(&self) -> ForkLocator {
        ForkLocator {
            validator_address: self.validator_address().clone(),
            block_number: self.block_number(),
        }
    }

    /// Address of the offending validator.
    pub fn validator_address(&self) -> &Address {
        &self.validator_address
    }
    /// Block number at which the offense occurred.
    pub fn block_number(&self) -> u32 {
        self.header1.block_number
    }
    /// Hash of header number 1.
    pub fn header1_hash(&self) -> Blake2bHash {
        self.header1.hash()
    }
    /// Hash of header number 2.
    pub fn header2_hash(&self) -> Blake2bHash {
        self.header2.hash()
    }

    /// Verify the validity of a fork proof.
    ///
    /// Does not verify the validator address.
    pub fn verify_excluding_address(
        &self,
        signing_key: &SchnorrPublicKey,
    ) -> Result<(), EquivocationProofError> {
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
        std::hash::Hash::hash(&self.locator(), state)
    }
}

impl SerializeContent for ForkProof {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        self.locator().serialize_content::<_, H>(writer)
    }
}

/// Possible equivocation proof validation errors.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EquivocationProofError {
    #[error("Slot mismatch")]
    SlotMismatch,
    #[error("No overlap between signer sets")]
    NoOverlap,
    #[error("Invalid justification")]
    InvalidJustification,
    #[error("Invalid validator address")]
    InvalidValidatorAddress,
    #[error("Same header")]
    SameHeader,
    #[error("Wrong order")]
    WrongOrder,
}

/// Struct representing a double proposal proof. A double proposal proof proves that a given
/// validator created two macro block proposals at the same height, in the same round.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DoubleProposalProof {
    /// Address of the offending validator.
    validator_address: Address,
    /// Header number 1.
    header1: MacroHeader,
    /// Header number 2.
    header2: MacroHeader,
    /// Justification for header number 1.
    justification1: SchnorrSignature,
    /// Justification for header number 2.
    justification2: SchnorrSignature,
}

impl DoubleProposalProof {
    /// The maximum size of a double proposal proof.
    pub const MAX_SIZE: usize =
        Address::SIZE + 2 * MacroHeader::MAX_SIZE + 2 * SchnorrSignature::SIZE;

    pub fn new(
        validator_address: Address,
        mut header1: MacroHeader,
        mut justification1: SchnorrSignature,
        mut header2: MacroHeader,
        mut justification2: SchnorrSignature,
    ) -> DoubleProposalProof {
        let hash1: Blake2bHash = header1.hash();
        let hash2: Blake2bHash = header2.hash();
        if hash1 > hash2 {
            mem::swap(&mut header1, &mut header2);
            mem::swap(&mut justification1, &mut justification2);
        }
        DoubleProposalProof {
            validator_address,
            header1,
            header2,
            justification1,
            justification2,
        }
    }

    /// Locator of this proof.
    ///
    /// It is used to check that only one double proposal proof can be included
    /// for a given block height and round.
    pub fn locator(&self) -> DoubleProposalLocator {
        DoubleProposalLocator {
            validator_address: self.validator_address().clone(),
            block_number: self.block_number(),
            round: self.round(),
        }
    }

    /// Address of the offending validator.
    pub fn validator_address(&self) -> &Address {
        &self.validator_address
    }
    /// Block number at which the offense occurred.
    pub fn block_number(&self) -> u32 {
        self.header1.block_number
    }
    /// Round of the proposals.
    pub fn round(&self) -> u32 {
        self.header1.round
    }
    /// Hash of header number 1.
    pub fn header1_hash(&self) -> Blake2bHash {
        self.header1.hash()
    }
    /// Hash of header number 2.
    pub fn header2_hash(&self) -> Blake2bHash {
        self.header2.hash()
    }

    /// Verify the validity of a double proposal proof.
    pub fn verify_excluding_address(
        &self,
        signing_key: &SchnorrPublicKey,
    ) -> Result<(), EquivocationProofError> {
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
        std::hash::Hash::hash(&self.locator(), state)
    }
}

impl SerializeContent for DoubleProposalProof {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        self.locator().serialize_content::<_, H>(writer)
    }
}

/// Struct representing a double vote proof. A double vote proof proves that a given
/// validator voted twice at same height, in the same round.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DoubleVoteProof {
    /// Address of the offending validator.
    validator_address: Address,
    /// Round and block height information.
    tendermint_id: TendermintIdentifier,
    /// Hash of proposal number 1.
    proposal_hash1: Option<Blake2sHash>,
    /// Hash of proposal number 2.
    proposal_hash2: Option<Blake2sHash>,
    /// Aggregate signature for proposal 1.
    signature1: AggregateSignature,
    /// Aggregate signature for proposal 2.
    signature2: AggregateSignature,
    /// Signers for proposal 1.
    signers1: BitSet,
    /// Signers for proposal 2.
    signers2: BitSet,
}

impl DoubleVoteProof {
    /// The maximum size of a double proposal proof.
    pub const MAX_SIZE: usize = 2 * MacroHeader::MAX_SIZE
        + 2 * nimiq_serde::option_max_size(Blake2sHash::SIZE)
        + 2 * AggregateSignature::SIZE
        + 2 * BitSet::max_size(Policy::SLOTS as usize);

    pub fn new(
        tendermint_id: TendermintIdentifier,
        validator_address: Address,
        mut proposal_hash1: Option<Blake2sHash>,
        mut signature1: AggregateSignature,
        mut signers1: BitSet,
        mut proposal_hash2: Option<Blake2sHash>,
        mut signature2: AggregateSignature,
        mut signers2: BitSet,
    ) -> DoubleVoteProof {
        if proposal_hash1 > proposal_hash2 {
            mem::swap(&mut proposal_hash1, &mut proposal_hash2);
            mem::swap(&mut signature1, &mut signature2);
            mem::swap(&mut signers1, &mut signers2);
        }
        DoubleVoteProof {
            tendermint_id,
            validator_address,
            proposal_hash1,
            proposal_hash2,
            signature1,
            signature2,
            signers1,
            signers2,
        }
    }

    /// Locator of this proof.
    ///
    /// It is used to check that only one double vote proof can be included for
    /// a given block height and round.
    pub fn locator(&self) -> DoubleVoteLocator {
        DoubleVoteLocator {
            validator_address: self.validator_address().clone(),
            block_number: self.block_number(),
            round: self.round(),
            step: self.step(),
        }
    }

    /// Address of the offending validator.
    pub fn validator_address(&self) -> &Address {
        &self.validator_address
    }
    /// Block number at which the offense occurred.
    pub fn block_number(&self) -> u32 {
        self.tendermint_id.block_number
    }
    /// Round number in which the offense occurred.
    pub fn round(&self) -> u32 {
        self.tendermint_id.round_number
    }
    /// Tendermint step in which the offense occurred.
    pub fn step(&self) -> TendermintStep {
        self.tendermint_id.step
    }

    /// Verify the validity of a double vote proof.
    ///
    /// The parameter `validators` must be the historic validators of that
    /// macro block production.
    pub fn verify_excluding_address(
        &self,
        validators: &Validators,
        validator_slots: Range<u16>,
    ) -> Result<(), EquivocationProofError> {
        // Check that the proposals are not equal and in the right order:
        match self.proposal_hash1.cmp(&self.proposal_hash2) {
            Ordering::Less => {}
            Ordering::Equal => return Err(EquivocationProofError::SameHeader),
            Ordering::Greater => return Err(EquivocationProofError::WrongOrder),
        }

        // Check that at least one of the validator's slots is actually contained in both signer sets.
        #[allow(clippy::redundant_clone)]
        if !validator_slots
            .clone()
            .any(|s| self.signers1.contains(s as usize) && self.signers2.contains(s as usize))
        {
            return Err(EquivocationProofError::NoOverlap);
        }

        let verify =
            |proposal_hash, signers: &BitSet, signature| -> Result<(), EquivocationProofError> {
                // Calculate the message that was actually signed by the validators.
                let message = TendermintVote {
                    proposal_hash,
                    id: self.tendermint_id.clone(),
                };
                // Verify the signatures.
                let mut agg_pk = AggregatePublicKey::new();
                for (i, pk) in validators.voting_keys().iter().enumerate() {
                    if signers.contains(i) {
                        agg_pk.aggregate(pk);
                    }
                }
                if !agg_pk.verify(&message, signature) {
                    return Err(EquivocationProofError::InvalidJustification);
                }
                Ok(())
            };

        verify(
            self.proposal_hash1.clone(),
            &self.signers1,
            &self.signature1,
        )?;
        verify(
            self.proposal_hash2.clone(),
            &self.signers2,
            &self.signature2,
        )?;
        Ok(())
    }
}

impl std::hash::Hash for DoubleVoteProof {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.locator(), state)
    }
}

impl SerializeContent for DoubleVoteProof {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        self.locator().serialize_content::<_, H>(writer)
    }
}

#[cfg(test)]
mod test {
    use nimiq_bls::{AggregateSignature, SecretKey};
    use nimiq_collections::BitSet;
    use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, HashOutput};
    use nimiq_keys::{Address, KeyPair, PrivateKey};
    use nimiq_primitives::{policy::Policy, TendermintIdentifier, TendermintStep, TendermintVote};
    use nimiq_serde::Deserialize;
    use nimiq_vrf::VrfSeed;

    use crate::{
        DoubleProposalProof, DoubleVoteProof, EquivocationProof, ForkProof, MacroHeader,
        MicroHeader,
    };

    #[test]
    fn fork_proof_sort_key() {
        let key = KeyPair::from(
            PrivateKey::from_bytes(
                &hex::decode(&"8a535c4be49186007503231c8569f873eb512eae308d9be7e7de20af1ddc1663")
                    .unwrap(),
            )
            .unwrap(),
        );

        let header1 = MicroHeader {
            version: Policy::VERSION,
            block_number: Policy::genesis_block_number(),
            timestamp: 0,
            parent_hash: Blake2bHash::default(),
            seed: VrfSeed::default(),
            extra_data: vec![],
            state_root: Blake2bHash::default(),
            body_root: Blake2sHash::default(),
            diff_root: Blake2bHash::default(),
            history_root: Blake2bHash::default(),
        };
        let header2 = MicroHeader {
            version: 1234,
            block_number: Policy::genesis_block_number(),
            timestamp: 1,
            parent_hash: "".hash(),
            seed: VrfSeed::default().sign_next(&key),
            extra_data: vec![1],
            state_root: "".hash(),
            body_root: "".hash(),
            diff_root: "".hash(),
            history_root: "".hash(),
        };
        let header3 = MicroHeader {
            version: 4321,
            block_number: Policy::genesis_block_number(),
            timestamp: 2,
            parent_hash: "1".hash(),
            seed: VrfSeed::default().sign_next(&key).sign_next(&key),
            extra_data: vec![2],
            state_root: "1".hash(),
            body_root: "1".hash(),
            diff_root: "1".hash(),
            history_root: "1".hash(),
        };

        // Headers from a different block height.
        let mut header4 = header1.clone();
        header4.block_number += 1;
        let mut header5 = header2.clone();
        header5.block_number += 1;

        let justification1 = key.sign(header1.hash::<Blake2bHash>().as_bytes());
        let justification2 = key.sign(header2.hash::<Blake2bHash>().as_bytes());
        let justification3 = key.sign(header3.hash::<Blake2bHash>().as_bytes());
        let justification4 = key.sign(header4.hash::<Blake2bHash>().as_bytes());
        let justification5 = key.sign(header5.hash::<Blake2bHash>().as_bytes());

        let proof1: EquivocationProof = ForkProof::new(
            Address::burn_address(),
            header1.clone(),
            justification1.clone(),
            header2.clone(),
            justification2.clone(),
        )
        .into();
        let proof2: EquivocationProof = ForkProof::new(
            Address::burn_address(),
            header2.clone(),
            justification2.clone(),
            header3.clone(),
            justification3.clone(),
        )
        .into();
        let proof3: EquivocationProof = ForkProof::new(
            Address::burn_address(),
            header3.clone(),
            justification3.clone(),
            header1.clone(),
            justification1.clone(),
        )
        .into();
        // Different block height.
        let proof4: EquivocationProof = ForkProof::new(
            Address::burn_address(),
            header4.clone(),
            justification4.clone(),
            header5.clone(),
            justification5.clone(),
        )
        .into();

        assert_eq!(proof1.sort_key(), proof2.sort_key());
        assert_eq!(proof1.sort_key(), proof3.sort_key());
        assert_ne!(proof1.sort_key(), proof4.sort_key());
    }

    #[test]
    fn double_proposal_proof_sort_key() {
        let key = KeyPair::from(
            PrivateKey::from_bytes(
                &hex::decode(&"8a535c4be49186007503231c8569f873eb512eae308d9be7e7de20af1ddc1663")
                    .unwrap(),
            )
            .unwrap(),
        );

        let header1 = MacroHeader {
            version: Policy::VERSION,
            block_number: Policy::genesis_block_number(),
            round: 0,
            timestamp: 0,
            parent_hash: Blake2bHash::default(),
            parent_election_hash: Blake2bHash::default(),
            interlink: None,
            seed: VrfSeed::default(),
            extra_data: vec![],
            state_root: Blake2bHash::default(),
            body_root: Blake2sHash::default(),
            diff_root: Blake2bHash::default(),
            history_root: Blake2bHash::default(),
        };
        let header2 = MacroHeader {
            version: 1234,
            block_number: Policy::genesis_block_number(),
            round: 0,
            timestamp: 1,
            parent_hash: "".hash(),
            parent_election_hash: "".hash(),
            interlink: Some(vec![]),
            seed: VrfSeed::default().sign_next(&key),
            extra_data: vec![1],
            state_root: "".hash(),
            body_root: "".hash(),
            diff_root: "".hash(),
            history_root: "".hash(),
        };
        let header3 = MacroHeader {
            version: 4321,
            block_number: Policy::genesis_block_number(),
            round: 0,
            timestamp: 2,
            parent_hash: "1".hash(),
            parent_election_hash: "1".hash(),
            interlink: Some(vec![Blake2bHash::default()]),
            seed: VrfSeed::default().sign_next(&key).sign_next(&key),
            extra_data: vec![2],
            state_root: "1".hash(),
            body_root: "1".hash(),
            diff_root: "1".hash(),
            history_root: "1".hash(),
        };

        // Headers from a different block height.
        let mut header4 = header1.clone();
        header4.block_number += 1;
        let mut header5 = header2.clone();
        header5.block_number += 1;

        // Headers from a different round.
        let mut header6 = header1.clone();
        header6.round += 1;
        let mut header7 = header2.clone();
        header7.round += 1;

        let justification1 = key.sign(header1.hash::<Blake2bHash>().as_bytes());
        let justification2 = key.sign(header2.hash::<Blake2bHash>().as_bytes());
        let justification3 = key.sign(header3.hash::<Blake2bHash>().as_bytes());
        let justification4 = key.sign(header4.hash::<Blake2bHash>().as_bytes());
        let justification5 = key.sign(header5.hash::<Blake2bHash>().as_bytes());
        let justification6 = key.sign(header6.hash::<Blake2bHash>().as_bytes());
        let justification7 = key.sign(header7.hash::<Blake2bHash>().as_bytes());

        let proof1: EquivocationProof = DoubleProposalProof::new(
            Address::burn_address(),
            header1.clone(),
            justification1.clone(),
            header2.clone(),
            justification2.clone(),
        )
        .into();
        let proof2: EquivocationProof = DoubleProposalProof::new(
            Address::burn_address(),
            header2.clone(),
            justification2.clone(),
            header3.clone(),
            justification3.clone(),
        )
        .into();
        let proof3: EquivocationProof = DoubleProposalProof::new(
            Address::burn_address(),
            header3.clone(),
            justification3.clone(),
            header1.clone(),
            justification1.clone(),
        )
        .into();
        // Different block height.
        let proof4: EquivocationProof = DoubleProposalProof::new(
            Address::burn_address(),
            header4.clone(),
            justification4.clone(),
            header5.clone(),
            justification5.clone(),
        )
        .into();
        // Different round.
        let proof5: EquivocationProof = DoubleProposalProof::new(
            Address::burn_address(),
            header6.clone(),
            justification6.clone(),
            header7.clone(),
            justification7.clone(),
        )
        .into();

        assert_eq!(proof1.sort_key(), proof2.sort_key());
        assert_eq!(proof1.sort_key(), proof3.sort_key());
        assert_ne!(proof1.sort_key(), proof4.sort_key());
        assert_ne!(proof1.sort_key(), proof5.sort_key());
        assert_ne!(proof4.sort_key(), proof5.sort_key());
    }

    #[test]
    fn double_vote_proof_sort_key() {
        let key = SecretKey::deserialize_from_vec(&hex::decode(&"f82ec9bb46d94c6bd6272d55e08fa5bfb911257136d205d7ee41c4f8fd839cf7a38e156c23d03c10597a4faa52a28a638756cdd83355db3eaafe37c5a94cc5c062f6de73890a6387175e579ef0083e3fe516dc61c2fab73e2183ea58a0b700").unwrap()).unwrap();

        let signers: BitSet = [0].into_iter().collect();

        let proposal1 = None;
        let proposal2 = Some(Blake2sHash::default());
        let proposal3 = Some("".hash());

        let id = TendermintIdentifier {
            block_number: 1,
            round_number: 2,
            step: TendermintStep::PreVote,
        };

        let other_id1 = TendermintIdentifier {
            block_number: 2,
            round_number: 2,
            step: TendermintStep::PreVote,
        };

        let other_id2 = TendermintIdentifier {
            block_number: 1,
            round_number: 3,
            step: TendermintStep::PreVote,
        };

        let other_id3 = TendermintIdentifier {
            block_number: 1,
            round_number: 2,
            step: TendermintStep::PreCommit,
        };

        let other_id4 = TendermintIdentifier {
            block_number: 1,
            round_number: 2,
            step: TendermintStep::Propose,
        };

        let signature1 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal1.clone(),
            id: id.clone(),
        })]);
        let signature2 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal2.clone(),
            id: id.clone(),
        })]);
        let signature3 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal3.clone(),
            id: id.clone(),
        })]);
        let signature4 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal1.clone(),
            id: other_id1.clone(),
        })]);
        let signature5 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal2.clone(),
            id: other_id1.clone(),
        })]);
        let signature6 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal1.clone(),
            id: other_id2.clone(),
        })]);
        let signature7 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal2.clone(),
            id: other_id2.clone(),
        })]);
        let signature8 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal1.clone(),
            id: other_id3.clone(),
        })]);
        let signature9 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal2.clone(),
            id: other_id3.clone(),
        })]);
        let signature10 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal1.clone(),
            id: other_id4.clone(),
        })]);
        let signature11 = AggregateSignature::from_signatures(&[key.sign(&TendermintVote {
            proposal_hash: proposal2.clone(),
            id: other_id4.clone(),
        })]);

        let proof1: EquivocationProof = DoubleVoteProof::new(
            id.clone(),
            Address::burn_address(),
            proposal1.clone(),
            signature1,
            signers.clone(),
            proposal2.clone(),
            signature2.clone(),
            signers.clone(),
        )
        .into();
        let proof2: EquivocationProof = DoubleVoteProof::new(
            id.clone(),
            Address::burn_address(),
            proposal2.clone(),
            signature2,
            signers.clone(),
            proposal3.clone(),
            signature3.clone(),
            signers.clone(),
        )
        .into();
        let proof3: EquivocationProof = DoubleVoteProof::new(
            id.clone(),
            Address::burn_address(),
            proposal3.clone(),
            signature3,
            signers.clone(),
            proposal1.clone(),
            signature1.clone(),
            signers.clone(),
        )
        .into();
        // Different block height.
        let proof4: EquivocationProof = DoubleVoteProof::new(
            other_id1,
            Address::burn_address(),
            proposal1.clone(),
            signature4,
            signers.clone(),
            proposal2.clone(),
            signature5.clone(),
            signers.clone(),
        )
        .into();
        // Different round.
        let proof5: EquivocationProof = DoubleVoteProof::new(
            other_id2,
            Address::burn_address(),
            proposal1.clone(),
            signature6,
            signers.clone(),
            proposal2.clone(),
            signature7.clone(),
            signers.clone(),
        )
        .into();
        // Different step, 1.
        let proof6: EquivocationProof = DoubleVoteProof::new(
            other_id3,
            Address::burn_address(),
            proposal1.clone(),
            signature8,
            signers.clone(),
            proposal2.clone(),
            signature9.clone(),
            signers.clone(),
        )
        .into();
        // Different step, 2.
        let proof7: EquivocationProof = DoubleVoteProof::new(
            other_id4,
            Address::burn_address(),
            proposal1.clone(),
            signature10,
            signers.clone(),
            proposal2.clone(),
            signature11.clone(),
            signers.clone(),
        )
        .into();

        assert_eq!(proof1.sort_key(), proof2.sort_key());
        assert_eq!(proof1.sort_key(), proof3.sort_key());
        assert_ne!(proof1.sort_key(), proof4.sort_key());
        assert_ne!(proof1.sort_key(), proof5.sort_key());
        assert_ne!(proof1.sort_key(), proof6.sort_key());
        assert_ne!(proof1.sort_key(), proof7.sort_key());

        let mut different = vec![proof4, proof5, proof6, proof7];
        let num_different = different.len();
        different.sort_by(|p1, p2| p1.sort_key().cmp(&p2.sort_key()));
        different.dedup_by(|p1, p2| p1.sort_key() == p2.sort_key());
        assert_eq!(different.len(), num_different);
    }
}
