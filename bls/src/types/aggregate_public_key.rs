use std::fmt;

use algebra::mnt6_753::G2Projective;
use num_traits::Zero;

use nimiq_hash::Hash;

use crate::{AggregateSignature, PublicKey, SigHash};

/// An aggregate public key. Mathematically, it is equivalent to a regular public key. However, we created a new type for it in order to help differentiate between the two use cases.
#[derive(Clone, Copy)]
pub struct AggregatePublicKey(pub(crate) PublicKey);

impl AggregatePublicKey {
    /// Creates a new "empty" aggregate public key. It is simply the identity element of the elliptic curve, also known as the point at infinity.
    pub fn new() -> Self {
        AggregatePublicKey(PublicKey {
            public_key: G2Projective::zero(),
        })
    }

    /// Creates an aggregated public key from an array of regular public keys.
    /// When using this method, it is essential that there exist proofs of knowledge of the secret key for each public key.
    /// Otherwise, an adversary can submit a public key to cancel out other public keys. This is called a "rogue key attack".
    pub fn from_public_keys(public_keys: &[PublicKey]) -> Self {
        let mut agg_key = G2Projective::zero();
        for x in public_keys {
            agg_key += &x.public_key;
        }
        return AggregatePublicKey(PublicKey {
            public_key: agg_key,
        });
    }

    /// Adds a single regular public key to an aggregated public keys.
    /// When using this method, it is essential that there exist proofs of knowledge of the secret key for each public key.
    /// Otherwise, an adversary can submit a public key to cancel out other public keys. This is called a "rogue key attack".
    pub fn aggregate(&mut self, key: &PublicKey) {
        self.0.public_key += &key.public_key;
    }

    /// Merges two aggregated public keys.
    /// When using this method, it is essential that there exist proofs of knowledge of the secret key for each public key.
    /// Otherwise, an adversary can submit a public key to cancel out other public keys. This is called a "rogue key attack".
    pub fn merge_into(&mut self, other: &Self) {
        self.0.public_key += &other.0.public_key;
    }

    /// Verifies an aggregate signature, over the same message, given that message.
    pub fn verify<M: Hash>(&self, msg: &M, signature: &AggregateSignature) -> bool {
        self.0.verify::<M>(msg, &signature.0)
    }

    /// Verifies an aggregate signature, over the same message, given the hash of that message.
    pub fn verify_hash(&self, hash: SigHash, signature: &AggregateSignature) -> bool {
        self.0.verify_hash(hash, &signature.0)
    }
}

impl Eq for AggregatePublicKey {}

impl PartialEq for AggregatePublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Default for AggregatePublicKey {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AggregatePublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for AggregatePublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Debug::fmt(&self.0, f)
    }
}
