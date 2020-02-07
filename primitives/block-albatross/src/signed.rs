use std::fmt::Debug;
use std::marker::PhantomData;

use beserial::{Deserialize, Serialize, WriteBytesExt};
use bls::SigHash;
use bls::{AggregatePublicKey, AggregateSignature, PublicKey, SecretKey, Signature};
use collections::bitset::BitSet;
use hash::{Blake2sHasher, Hasher, SerializeContent};
use primitives::slot::{SlotBand, SlotCollection, SlotIndex, ValidatorSlots};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedMessage<M: Message> {
    // the signed message
    //  - view change: (VIEW-CHANGE, i + 1, b)
    //    - i: current view change number
    //    - b: current block number
    //  - pbft-prepare: (PREPARE, h)
    //  - pbft-commit: (COMMIT, h)
    //    - h: block hash -> Blake2bHash
    // X the actual message doesn't contain the prefix. This is added only during signing
    pub message: M,

    // index of public key of signer
    // XXX they need to be indexable, because we will include a bitmap of all signers in the block
    pub signer_idx: u16,

    // signature over message
    pub signature: Signature,
}

impl<M: Message> SignedMessage<M> {
    /// Verify signed message
    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key.verify_hash(self.message.hash_with_prefix(), &self.signature)
    }

    /// Create SignedMessage from message.
    pub fn from_message(message: M, secret_key: &SecretKey, signer_idx: u16) -> Self {
        let signature = message.sign(secret_key);
        Self {
            message,
            signer_idx,
            signature,
        }
    }
}

// XXX The contents of ViewChangeMessage and PbftMessage (and any other message that is signed by
// a validator) must be distinguishable!
// Therefore all signed messages should be prefixed with a standardized type. We should keep those
// prefixed at one place to not accidentally create collisions.

/// prefix to sign view change messages
pub const PREFIX_VIEW_CHANGE: u8 = 0x01;
/// prefix to sign a pbft-proposal
pub const PREFIX_PBFT_PROPOSAL: u8 = 0x02;
/// prefix to sign pbft-prepare messages
pub const PREFIX_PBFT_PREPARE: u8 = 0x03;
/// prefix to sign pbft-commit messages
pub const PREFIX_PBFT_COMMIT: u8 = 0x04;
/// prefix to sign proof of knowledge of secret key
pub const PREFIX_POKOSK: u8 = 0x05;
/// prefix to sign a validator info
pub const PREFIX_VALIDATOR_INFO: u8 = 0x06;

pub trait Message:
    Clone
    + Debug
    + Serialize
    + Deserialize
    + SerializeContent
    + Send
    + Sync
    + Sized
    + PartialEq
    + 'static
{
    const PREFIX: u8;

    fn hash_with_prefix(&self) -> SigHash {
        let mut h = Blake2sHasher::new();
        h.write_u8(Self::PREFIX)
            .expect("Failed to write prefix to hasher for signature.");
        self.serialize_content(&mut h)
            .expect("Failed to write message to hasher for signature.");
        h.finish()
    }

    fn sign(&self, secret_key: &SecretKey) -> Signature {
        secret_key.sign_hash(self.hash_with_prefix())
    }
}

pub enum AggregateError {
    Overlapping,
}

/// DEPRECATED: We don't need this anymore.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AggregateProofBuilder<M> {
    /// Indices of validators that signed this proof
    pub signers: BitSet,

    /// The aggregate public key of the signers
    pub public_key: AggregatePublicKey,

    /// The aggregate signature
    pub signature: AggregateSignature,

    /// The cumulative number of slots of the signers
    pub num_slots: u16,

    #[beserial(skip)]
    _message: PhantomData<M>,
}

impl<M: Message> AggregateProofBuilder<M> {
    pub fn new() -> Self {
        Self {
            signers: BitSet::with_capacity(0), // TODO: Fix this by the size of the active validator set
            public_key: AggregatePublicKey::new(),
            signature: AggregateSignature::new(),
            num_slots: 0,
            _message: PhantomData,
        }
    }

    pub fn contains(&self, signed: &SignedMessage<M>) -> bool {
        self.signers.contains(signed.signer_idx as usize)
    }

    /// Adds a signed message to an aggregate proof
    /// NOTE: This method assumes the signature of the message was already checked
    pub fn add_signature(
        &mut self,
        public_key: &PublicKey,
        num_slots: u16,
        signed: &SignedMessage<M>,
    ) -> bool {
        debug_assert!(signed.verify(public_key));
        let signer_idx = signed.signer_idx as usize;
        if self.signers.contains(signer_idx) {
            return false;
        }
        self.signers.insert(signer_idx);
        self.public_key.aggregate(public_key);
        self.signature.aggregate(&signed.signature);
        self.num_slots += num_slots;
        true
    }

    #[allow(unused_variables)]
    pub fn merge(&mut self, proof: &AggregateProof<M>) -> Result<Self, AggregateError> {
        unimplemented!()
    }

    pub fn verify(&self, message: &M, threshold: u16) -> Result<(), AggregateProofError> {
        if self.num_slots < threshold {
            return Err(AggregateProofError::InsufficientSigners(
                self.num_slots,
                threshold,
            ));
        }

        if !self
            .public_key
            .verify_hash(message.hash_with_prefix(), &self.signature)
        {
            return Err(AggregateProofError::InvalidSignature);
        }

        Ok(())
    }

    pub fn clear(&mut self) {
        self.signers.clear();
        self.public_key = AggregatePublicKey::new();
        self.signature = AggregateSignature::new();
        self.num_slots = 0;
    }

    pub fn build(self) -> AggregateProof<M> {
        AggregateProof {
            signers: self.signers,
            signature: self.signature,
            _message: PhantomData,
        }
    }
}

impl<M: Message> Default for AggregateProofBuilder<M> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
/// TODO: Maybe refactor this, since we only construct those from Handel MultiSignatures now.
pub struct AggregateProof<M: Message> {
    /// Indices of validators that signed this proof
    pub signers: BitSet,

    /// The aggregate signature
    pub signature: AggregateSignature,

    #[beserial(skip)]
    _message: PhantomData<M>,
}

impl<M: Message> AggregateProof<M> {
    pub fn new(signature: AggregateSignature, signers: BitSet) -> Self {
        Self {
            signature,
            signers,
            _message: PhantomData,
        }
    }

    pub fn votes(&self, validators: &ValidatorSlots) -> Result<u16, AggregateProofError> {
        votes_for_signers(validators, &self.signers)
    }

    /// Verify message against aggregate signature and check the required number of signatures.
    /// Expects valid validator public keys.
    pub fn verify(
        &self,
        message: &M,
        validators: &ValidatorSlots,
        threshold: u16,
    ) -> Result<(), AggregateProofError> {
        // Aggregate signatures and count votes
        let mut public_key = AggregatePublicKey::new();
        let mut votes = 0;
        for signer_idx in self.signers.iter() {
            let validator = validators
                .get_by_band_number(signer_idx as u16)
                .ok_or_else(|| AggregateProofError::InvalidSignerIndex(signer_idx as u16))?;
            public_key.aggregate(&validator.public_key().uncompress_unchecked());
            votes += validator.num_slots();
        }

        if votes < threshold {
            return Err(AggregateProofError::InsufficientSigners(votes, threshold));
        }

        if !public_key.verify_hash(message.hash_with_prefix(), &self.signature) {
            trace!("Invalid signature");
            return Err(AggregateProofError::InvalidSignature);
        }

        Ok(())
    }
}

pub fn votes_for_signers(
    validators: &ValidatorSlots,
    signers: &BitSet,
) -> Result<u16, AggregateProofError> {
    let mut votes = 0;
    for signer_idx in signers.iter() {
        votes += validators
            .get_num_slots(SlotIndex::Band(signer_idx as u16))
            .ok_or_else(|| AggregateProofError::InvalidSignerIndex(signer_idx as u16))?;
    }
    Ok(votes)
}

#[derive(Clone, Debug, PartialEq, Eq, Fail)]
pub enum AggregateProofError {
    #[fail(display = "Invalid signer index: {}", _0)]
    InvalidSignerIndex(u16),
    #[fail(display = "Invalid signature")]
    InvalidSignature,
    #[fail(display = "Insufficient signers (got {}, want {})", _0, _1)]
    InsufficientSigners(u16, u16),
}
