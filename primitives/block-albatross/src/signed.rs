use beserial::{Serialize, Deserialize, uvar};
use nimiq_bls::bls12_381::{Signature, SecretKey, PublicKey, AggregateSignature, AggregatePublicKey};
use nimiq_bls::SigHash;
use hash::{Blake2bHasher, SerializeContent, Hasher};
use beserial::WriteBytesExt;

use std::fmt::Debug;
use bitvec::{BitVec, BigEndian};
use std::marker::PhantomData;
use beserial::ToPrimitive;


// TODO: To use a binomial swap forest:
//  * instead of pk_idx use a bitvec
//  * instead of signature use a AggregateSignature
//  * (all, not just active) validators are identifiable by ID which has the common prefix metric
//  * The sender of a SignedValidatorMessage includes it's ID
//  * Add other messages (authenticated) for: Invite, Later, No


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
    pub pk_idx: uvar,

    // signature over message
    pub signature: Signature
}

impl<M: Message> SignedMessage<M> {
    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key.verify_hash(self.message.hash_with_prefix(), &self.signature)
    }
}


// XXX The contents of ViewChangeMessage and PbftMessage (and any other message that is signed by
// a validator) must be distinguishable!
// Therefore all signed messages should be prefixed with a standarized type. We should keep those
// prefixed at one place to not accidently create collisions.

// prefix to sign view change messages
pub const PREFIX_VIEW_CHANGE: u8 = 0x01;
// prefix to sign pbft-prepare messages
pub const PREFIX_PBFT_PREPARE: u8 = 0x02;
// prefix to sign pbft-commit messages
pub const PREFIX_PBFT_COMMIT: u8 = 0x03;
// prefix to sign proof of knowledge of secret key
pub const PREFIX_POKOSK: u8 = 0x04;

pub trait Message: Clone + Debug + Serialize + Deserialize + SerializeContent {
    const PREFIX: u8;

    fn hash_with_prefix(&self) -> SigHash {
        let mut h = Blake2bHasher::new();
        h.write_u8(Self::PREFIX).expect("Failed to write prefix to hasher for signature.");
        self.serialize_content(&mut h).expect("Failed to write message to hasher for signature.");
        h.finish()
    }

    fn sign(&self, secret_key: &SecretKey) -> Signature {
        secret_key.sign_hash(self.hash_with_prefix())
    }
}

pub enum AggregateError {
    Overlapping
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AggregateProof<M> {
    pub num_signers: u32,
    #[beserial(len_type(u32))]
    pub signers: BitVec<BigEndian, u8>,
    pub public_key: AggregatePublicKey,
    pub signature: AggregateSignature,
    #[beserial(skip)]
    _message: PhantomData<M>
}

impl<M: Message> AggregateProof<M> {
    pub fn new() -> Self {
        Self {
            num_signers: 0,
            signers: BitVec::new(),
            public_key: AggregatePublicKey::new(),
            signature: AggregateSignature::new(),
            _message: PhantomData,
        }
    }

    pub fn contains(&self, signed: &SignedMessage<M>) -> bool {
        let pk_idx = signed.pk_idx.to_usize().unwrap();
        self.signers.get(pk_idx).unwrap_or(false)
    }

    /// Adds a signed message to an aggregate proof
    /// NOTE: This method assumes the signature of the message was already checked
    pub fn add_signature(&mut self, public_key: &PublicKey, signed: &SignedMessage<M>) {
        debug_assert!(signed.verify(public_key));
        let pk_idx = signed.pk_idx.to_usize().unwrap();
        if !self.signers.get(pk_idx).unwrap_or(false) {
            if pk_idx >= self.signers.len() {
                self.signers.resize(pk_idx, false);
            }
            self.num_signers += 1;
            self.signers.set(pk_idx, true);
            self.public_key.aggregate(public_key);
            self.signature.aggregate(&signed.signature);
        }
    }

    pub fn merge(&mut self, proof: &AggregateProof<M>) -> Result<(), AggregateError> {
        unimplemented!("TODO: Check if this is implemented correctly");
//        if (self.signers & proof.signers).any() {
//            return Err(AggregateError::Overlapping)
//        }
//        self.num_signers += proof.num_signers;
//        self.signers |= &proof.signers;
//        self.public_key.merge_into(&proof.public_key);
//        self.signature.merge_into(&proof.signature);
//        Ok(())
    }

    // Verify message against aggregate signature and optionally check if a threshold was reached
    pub fn verify(&self, message: &M, threshold: Option<usize>) -> bool {
        self.signers.count_ones() >= threshold.unwrap_or(0)
            && self.public_key.verify_hash(message.hash_with_prefix(), &self.signature)
    }
}
