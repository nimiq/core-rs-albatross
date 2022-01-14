use std::{cmp::Ordering, fmt};

use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_ff::Zero;
use ark_mnt6_753::{G1Projective, G2Projective, MNT6_753};
use log::error;

use nimiq_hash::Hash;

use crate::compression::BeSerialize;
use crate::{CompressedPublicKey, SecretKey, SigHash, Signature};

#[derive(Clone, Copy)]
pub struct PublicKey {
    /// The projective form is the longer one, with three coordinates. The affine form is the
    /// shorter one, with only two coordinates. Calculation is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support arithmetic with it.
    pub public_key: G1Projective,
}

impl PublicKey {
    /// Generates a public key from a given point in G1. This function will produce an error if it
    /// is given the point at infinity.
    pub fn new(public_key: G1Projective) -> Self {
        if public_key.is_zero() {
            error!("Public key cannot be the point at infinity!");
        }
        PublicKey { public_key }
    }

    /// Derives a public key from a secret key. This function will produce an error if it is given
    /// zero as an input.
    pub fn from_secret(x: &SecretKey) -> Self {
        let mut pk = G1Projective::prime_subgroup_generator();
        pk *= x.secret_key;
        Self::new(pk)
    }

    /// Verifies a signature given the signature and the message.
    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
        self.verify_hash(msg.hash(), signature)
    }

    /// Verifies a signature given the signature and the hash.
    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        self.verify_point(Signature::hash_to_point(hash), signature)
    }

    /// Verifies a signature given the signature and the elliptic curve point. This function will
    /// always return false if the public key is the point at infinity.
    pub fn verify_point(&self, hash_point: G2Projective, signature: &Signature) -> bool {
        if self.public_key.is_zero() {
            return false;
        }

        let lhs = MNT6_753::pairing(
            G1Projective::prime_subgroup_generator(),
            signature.signature,
        );

        let rhs = MNT6_753::pairing(self.public_key, hash_point);

        lhs == rhs
    }

    /// Transforms a public key into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine form),
    /// one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedPublicKey {
        let mut buffer = [0u8; CompressedPublicKey::SIZE];
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
    /// We order points according to their compressed (affine) encoding. The ordering is
    /// lexicographical and considers (in order):
    /// 1. Infinity
    /// 2. "Parity" bit of the y coordinate
    /// 3. x coordinate
    fn cmp(&self, other: &Self) -> Ordering {
        // Convert keys to affine form.
        let first = self.public_key.into_affine();
        let second = other.public_key.into_affine();

        // Check if any of the points is at infinity. If not continue.
        match (first.is_zero(), second.is_zero()) {
            (true, true) => return Ordering::Equal,
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            _ => {}
        }

        // Calculate the parity bits of the y coordinates.
        let y_bit_first = first.y > -first.y;
        let y_bit_second = second.y > -second.y;

        // Compare the parity bits. If they are equal, continue.
        match (y_bit_first, y_bit_second) {
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            _ => {}
        }

        // Finally, simply compare the x coordinates.
        first.x.cmp(&second.x)
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
