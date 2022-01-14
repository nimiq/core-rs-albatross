#[cfg(feature = "beserial")]
use std::fmt;

use ark_ff::{UniformRand, Zero};
use ark_mnt6_753::{Fr, G2Projective};

#[cfg(feature = "beserial")]
use beserial::Serialize;
use nimiq_hash::Hash;
use nimiq_utils::key_rng::SecureGenerate;
use nimiq_utils::key_rng::{CryptoRng, RngCore};

use crate::{CompressedSignature, SigHash, Signature};

#[derive(Clone, Copy)]
pub struct SecretKey {
    /// This is simply a number in the finite field Fr.
    /// Fr is also the prime subgroup of the elliptic curve.
    pub secret_key: Fr,
}

impl SecretKey {
    pub const SIZE: usize = 96;

    /// Creates a signature given a message.
    pub fn sign<M: Hash>(&self, msg: &M) -> Signature {
        self.sign_hash(msg.hash())
    }

    /// Creates a signature given a hash.
    pub fn sign_hash(&self, hash: SigHash) -> Signature {
        self.sign_point(Signature::hash_to_point(hash))
    }

    /// Creates a signature given a G2 point.
    pub fn sign_point(&self, hash_curve: G2Projective) -> Signature {
        let mut sig = hash_curve;
        sig *= self.secret_key;
        Signature {
            signature: sig,
            compressed: CompressedSignature::from(sig),
        }
    }
}

impl SecureGenerate for SecretKey {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let mut x = Fr::rand(rng);
        loop {
            if !x.is_zero() {
                break;
            }
            x = Fr::rand(rng);
        }
        SecretKey { secret_key: x }
    }
}

impl Eq for SecretKey {}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.secret_key.eq(&other.secret_key)
    }
}

#[cfg(feature = "beserial")]
impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "SecretKey({})", hex::encode(self.serialize_to_vec()))
    }
}

#[cfg(feature = "beserial")]
impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", hex::encode(self.serialize_to_vec()))
    }
}
