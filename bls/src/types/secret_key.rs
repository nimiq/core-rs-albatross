use std::{fmt, ops::MulAssign};

use algebra::mnt6_753::{Fr, G1Projective};
use algebra_core::UniformRand;
use num_traits::Zero;

use beserial::Serialize;
use nimiq_hash::Hash;
use nimiq_utils::key_rng::SecureGenerate;
use nimiq_utils::key_rng::{CryptoRng, Rng};

use crate::{SigHash, Signature};

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
        self.sign_g1(Signature::hash_to_g1(hash))
    }

    /// Creates a signature given a G1 point.
    pub fn sign_g1(&self, hash_curve: G1Projective) -> Signature {
        let mut sig = hash_curve;
        sig.mul_assign(self.secret_key);
        Signature { signature: sig }
    }
}

impl SecureGenerate for SecretKey {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self {
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
        f.write_str(&::hex::encode(self.serialize_to_vec()))
    }
}
