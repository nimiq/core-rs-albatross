use std::{cmp::Ordering, fmt, ops::MulAssign};

use algebra::mnt6_753::{G1Projective, G2Projective, MNT6_753};
use algebra_core::curves::{PairingEngine, ProjectiveCurve};
use log::error;
use num_traits::Zero;

use hash::Hash;
pub use utils::key_rng::{SecureGenerate, SecureRng};

use crate::compression::BeSerialize;
use crate::{CompressedPublicKey, SecretKey, SigHash, Signature};

#[derive(Clone, Copy)]
pub struct PublicKey {
    /// The projective form is the longer one, with three coordinates. The affine form is the shorter one, with only two coordinates. Calculation is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support arithmetic with it.
    pub public_key: G2Projective,
}

impl PublicKey {
    /// Generates a public key from a given point in G2. This function will produce an error if it is given the point at infinity.
    fn new(public_key: G2Projective) -> Self {
        if public_key.is_zero() {
            error!("Public key cannot be the point at infinity!");
        }
        PublicKey { public_key }
    }

    /// Derives a public key from a secret key. This function will produce an error if it is given zero as an input.
    pub fn from_secret(x: &SecretKey) -> Self {
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(x.secret_key);
        Self::new(pk)
    }

    /// Verifies a signature given the signature and the message.
    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
        self.verify_hash(msg.hash(), signature)
    }

    /// Verifies a signature given the signature and the hash.
    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        self.verify_g1(Signature::hash_to_g1(hash), signature)
    }

    /// Verifies a signature given the signature and the G1 point. This function will always return false if the public key is the point at infinity.
    pub fn verify_g1(&self, hash_curve: G1Projective, signature: &Signature) -> bool {
        if self.public_key.is_zero() {
            return false;
        }
        let lhs = MNT6_753::pairing(
            signature.signature,
            G2Projective::prime_subgroup_generator(),
        );
        let rhs = MNT6_753::pairing(hash_curve, self.public_key);
        lhs == rhs
    }

    /// Transforms a public key into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine form),
    /// one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedPublicKey {
        let mut buffer = [0u8; 288];
        BeSerialize::serialize(&self.public_key.into_affine(), &mut &mut buffer[..]).unwrap();
        CompressedPublicKey { public_key: buffer }
    }
}

impl Eq for PublicKey {}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.public_key.eq(&other.public_key)
    }
}

impl PartialOrd<PublicKey> for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.public_key
            .into_affine()
            .lexicographic_cmp(&other.public_key.into_affine())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PublicKey({})", self.compress().to_hex())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.compress().to_hex())
    }
}
