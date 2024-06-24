use std::{cmp::Ordering, fmt, ops::MulAssign};

use ark_ec::{pairing::Pairing, AffineRepr, CurveGroup, Group};
use ark_ff::Zero;
pub use ark_mnt6_753::G2Projective;
use ark_mnt6_753::{G1Projective, MNT6_753};
use ark_serialize::CanonicalSerialize;
use log::error;
use nimiq_hash::Hash;

use crate::{CompressedPublicKey, SecretKey, SigHash, Signature};

#[derive(Clone, Copy)]
#[cfg_attr(feature = "serde-derive", derive(nimiq_hash_derive::SerializeContent))]
pub struct PublicKey {
    /// The projective form is the longer one, with three coordinates. The affine form is the shorter one, with only two coordinates. Calculation is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support arithmetic with it.
    pub public_key: G2Projective,
}

impl PublicKey {
    /// Generates a public key from a given point in G2. This function will produce an error if it is given the point at infinity.
    pub fn new(public_key: G2Projective) -> Self {
        if public_key.is_zero() {
            error!("Public key cannot be the point at infinity!");
        }
        PublicKey { public_key }
    }

    /// Derives a public key from a secret key. This function will produce an error if it is given zero as an input.
    pub fn from_secret(x: &SecretKey) -> Self {
        let mut pk = G2Projective::generator();
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
        let lhs = MNT6_753::pairing(signature.signature, G2Projective::generator());
        let rhs = MNT6_753::pairing(hash_curve, self.public_key);
        lhs == rhs
    }

    /// Transforms a public key into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine form),
    /// one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedPublicKey {
        let mut buffer = [0u8; CompressedPublicKey::SIZE];
        CanonicalSerialize::serialize_compressed(
            &self.public_key.into_affine(),
            &mut &mut buffer[..],
        )
        .unwrap();
        CompressedPublicKey { public_key: buffer }
    }

    /// Multiplies a PublicKey by a u16. It's useful when you need to
    /// multiply a validator's public key by its number of slots.
    #[must_use]
    pub fn multiply(&self, x: u16) -> Self {
        let public_key = self.public_key.mul_bigint([x as u64]);
        PublicKey { public_key }
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

#[cfg(feature = "serde-derive")]
mod serde_derive {
    // TODO: Replace this with a generic serialization using `ToHex` and `FromHex`.
    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::{CompressedPublicKey, PublicKey};

    impl Serialize for PublicKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Serialize::serialize(&self.compress(), serializer)
        }
    }

    impl<'de> Deserialize<'de> for PublicKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            CompressedPublicKey::deserialize(deserializer)?
                .uncompress()
                .map_err(Error::custom)
        }
    }
}
