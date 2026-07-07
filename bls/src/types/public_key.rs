use std::{cmp::Ordering, fmt, ops::MulAssign, sync::OnceLock};

use ark_ec::{pairing::Pairing, AffineRepr, CurveGroup, PrimeGroup};
use ark_ff::Zero;
pub use ark_mnt6_753::G2Projective;
use ark_mnt6_753::{G1Projective, G2Affine, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use log::error;
use nimiq_hash::Hash;

use crate::{CompressedPublicKey, SecretKey, SigHash, Signature};

/// Returns the cached Miller loop coefficients of the `G2` generator. Preparing a `G2` point
/// runs the entire ate-loop point arithmetic, which is roughly half the cost of a Miller loop,
/// so computing it once for the fixed generator saves that work on every signature verification.
fn g2_generator_prepared() -> &'static <MNT6_753 as Pairing>::G2Prepared {
    static PREPARED: OnceLock<<MNT6_753 as Pairing>::G2Prepared> = OnceLock::new();
    PREPARED.get_or_init(|| G2Affine::generator().into())
}

#[derive(Clone, Copy)]
#[cfg_attr(feature = "serde-derive", derive(nimiq_hash_derive::SerializeContent))]
pub struct PublicKey {
    /// The projective form is the longer one, with three coordinates. The affine form is the shorter one, with only two coordinates. Calculation is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support arithmetic with it.
    pub public_key: G2Projective,
}

impl PublicKey {
    /// Size of the trusted serialization.
    ///
    /// See [`PublicKey::trusted_deserialize`].
    pub const TRUSTED_SERIALIZATION_SIZE: usize = 570;

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
        // The pairing equation `e(sig, g2) == e(H(m), pk)` is checked in its equivalent form
        // `e(sig, g2) * e(-H(m), pk) == 1`, which needs only one multi-Miller loop and one
        // final exponentiation instead of two full pairings. The Miller loop coefficients of
        // the fixed `g2` generator are precomputed once and shared by all verifications
        let miller_loop = MNT6_753::multi_miller_loop(
            [signature.signature, -hash_curve],
            [
                g2_generator_prepared().clone(),
                <MNT6_753 as Pairing>::G2Prepared::from(self.public_key),
            ],
        );
        // `Pairing::multi_pairing` is exactly this multi-Miller loop followed by the final
        // exponentiation, so it would read more cleanly. We spell it out instead because
        // `multi_pairing` `.unwrap()`s the final exponentiation: it would panic on a degenerate
        // (non-invertible) Miller loop output, whereas `signature` and `public_key` come off the
        // wire and an invalid input must be rejected as a `false`, not crash the node. Such
        // degenerate outputs cannot arise from a valid signature, so the explicit form is purely
        // defensive against malformed input
        MNT6_753::final_exponentiation(miller_loop).is_some_and(|pairing| pairing.is_zero())
    }

    /// Transforms a public key into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine form),
    /// one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedPublicKey {
        let mut buffer = [0u8; CompressedPublicKey::SIZE];
        let affine = self.public_key.into_affine();
        assert_eq!(affine.compressed_size(), buffer.len());
        affine.serialize_compressed(&mut buffer[..]).unwrap();
        CompressedPublicKey { public_key: buffer }
    }

    /// Multiplies a PublicKey by a u16. It's useful when you need to
    /// multiply a validator's public key by its number of slots.
    #[must_use]
    pub fn multiply(&self, x: u16) -> Self {
        let public_key = self.public_key.mul_bigint([x as u64]);
        PublicKey { public_key }
    }

    /// Serialization to trusted storage.
    ///
    /// If in doubt, use the default serialization, or even better, the default
    /// serialization of [`LazyPublicKey`](crate::lazy::LazyPublicKey) instead.
    ///
    /// See [`PublicKey::trusted_deserialize`] for details.
    pub fn trusted_serialize(&self) -> [u8; PublicKey::TRUSTED_SERIALIZATION_SIZE] {
        let mut result = [0u8; PublicKey::TRUSTED_SERIALIZATION_SIZE];
        assert_eq!(self.public_key.uncompressed_size(), result.len());
        self.public_key
            .serialize_uncompressed(&mut result[..])
            .unwrap();
        result
    }

    /// Deserialization from trusted storage.
    ///
    /// **This does not check whether the resulting key is on the curve.** This
    /// means that you can only use this serialization when you trust the
    /// creator of that serialization completely.
    ///
    /// If in doubt, use the default serialization, or even better, the default
    /// serialization of [`LazyPublicKey`](crate::lazy::LazyPublicKey) instead.
    pub fn trusted_deserialize(
        serialized: &[u8; PublicKey::TRUSTED_SERIALIZATION_SIZE],
    ) -> PublicKey {
        PublicKey {
            public_key: CanonicalDeserialize::deserialize_uncompressed_unchecked(&serialized[..])
                .unwrap(),
        }
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
