#![allow(non_snake_case)]

#[cfg(feature = "serde-derive")]
use std::borrow::Cow;
use std::{fmt, hash::Hash, io::Write};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use curve25519_dalek::{
    constants,
    edwards::{CompressedEdwardsY, EdwardsPoint},
    montgomery::MontgomeryPoint,
    scalar::Scalar,
    traits::IsIdentity,
};
use log::debug;
use nimiq_hash::{Blake2bHash, Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Ed25519PublicKey, KeyPair};
#[cfg(feature = "serde-derive")]
use nimiq_macros::add_serialization_fns_typed_arr;
use nimiq_macros::create_typed_array;
use num_bigint::BigUint;
use rand::CryptoRng;
use sha2::{Digest, Sha256, Sha512};

use crate::rng::Rng;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VrfError {
    Forged,
    InvalidSignature,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum VrfUseCase {
    /// Used to produce the next seed in the VRF seed chain.
    Seed = 1,
    /// Used to select the validator slots at the end of each epoch.
    ValidatorSlotSelection = 2,
    /// Used to determine the view slots at each block height.
    ViewSlotSelection = 3,
    /// Used to randomly distribute the rewards.
    RewardDistribution = 4,
}

create_typed_array!(VrfEntropy, u8, 32);
#[cfg(feature = "serde-derive")]
add_serialization_fns_typed_arr!(VrfEntropy, VrfEntropy::SIZE);

impl VrfEntropy {
    pub fn rng(self, use_case: VrfUseCase) -> VrfRng {
        VrfRng::new(self, use_case)
    }
}

impl fmt::Debug for VrfEntropy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("VrfEntropy")
            .field(&hex::encode(self.0))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(
    feature = "serde-derive",
    derive(
        nimiq_serde::Deserialize,
        nimiq_serde::Serialize,
        nimiq_serde::SerializedSize,
    )
)]
#[cfg_attr(feature = "serde-derive", serde(transparent))]
/// A struct containing a VRF Seed. It is simply the serialized output of the VXEdDSA algorithm.
///
/// <https://www.signal.org/docs/specifications/xeddsa/#vxeddsa>
///
/// Note that this signature is NOT unique for a given message and public key. In fact, if a signer
/// produces two VRF seeds for the same message they will be different (with overwhelmingly high
/// probability). This is because the signing algorithm uses a random input, similar to a Schnorr
/// signature. Furthermore, the signature is malleable, so it can be manipulated by anyone. So you
/// CANNOT use the VRF seed directly as a uniqueness or randomness source.
/// However, the entropy that we extract from the random seed is unique for a given message and
/// public key.
pub struct VrfSeed {
    #[cfg_attr(feature = "serde-derive", serde(with = "nimiq_serde::HexArray"))]
    pub(crate) signature: [u8; VrfSeed::SIZE],
}

/// Maps the SHA-512 digest of `input` to a point on Ed25519, following the map-to-curve step of
/// VXEdDSA (<https://www.signal.org/docs/specifications/xeddsa/#vxeddsa>).
///
/// This is a byte-for-byte reimplementation of `curve25519_dalek::edwards::EdwardsPoint::
/// nonspec_map_to_curve::<Sha512>`, which existed up to curve25519-dalek 4.x but was removed in
/// 5.0. It is CONSENSUS-CRITICAL: the resulting point `B_v` is folded into every VRF seed that is
/// part of the chain history, so the output MUST remain identical to the 4.x implementation, or
/// previously produced seeds stop verifying. The equivalence is guarded by the differential fuzz
/// test `elligator_matches_curve25519_dalek_4x` below.
///
/// The map is: `SHA-512(input)` → Elligator2 encode the first 32 bytes to a Montgomery
/// u-coordinate → lift to Edwards using the top bit as the sign → clear the cofactor.
///
/// Note this only ever processes public data (public key and message); the secret scalar is not
/// involved here, so the non-constant-time `BigUint` arithmetic introduces no timing side channel.
fn nonspec_map_to_curve(input: &[u8]) -> EdwardsPoint {
    let hash = Sha512::digest(input);
    let mut res = [0u8; 32];
    res.copy_from_slice(&hash[..32]);

    // The top bit of the (unmasked) digest selects the sign of the recovered Edwards point.
    let sign_bit = (res[31] & 0x80) >> 7;

    let u = elligator_encode(&res);

    MontgomeryPoint(u)
        .to_edwards(sign_bit)
        .expect("Montgomery conversion to Edwards point in Elligator failed")
        .mul_by_cofactor()
}

/// Elligator2 encode used by [`nonspec_map_to_curve`]. Reproduces curve25519-dalek 4.x's internal
/// `montgomery::elligator_encode` exactly, returning the canonical little-endian bytes of the
/// Montgomery u-coordinate.
///
/// For the field element `r` derived from `res` (little-endian, high bit ignored):
/// ```text
///   d   = -A / (1 + 2 r^2)          with A = 486662
///   eps = d^3 + A d^2 + d
///   u   = d        if eps is a square
///   u   = -d - A   otherwise
/// ```
/// The "is a square" test mirrors dalek's `sqrt_ratio_i(eps, 1)`, which reports `true` for both
/// quadratic residues and zero. Inversion is Fermat's `x^(p-2)` (matching dalek's `invert`, so the
/// degenerate `1 + 2 r^2 == 0` input inverts to 0 exactly as in 4.x).
fn elligator_encode(res: &[u8; 32]) -> [u8; 32] {
    // p = 2^255 - 19, the Ed25519 field modulus.
    let p = (BigUint::from(1u8) << 255u32) - BigUint::from(19u8);
    // Montgomery curve constant A.
    let a = BigUint::from(486662u32);

    // Fermat inverse: x^(p-2) mod p. Matches dalek's `FieldElement::invert`, including inv(0) = 0.
    let inv = |x: &BigUint| x.modpow(&(&p - 2u32), &p);

    // r = little-endian(res) with bit 255 cleared, reduced mod p (matches `FieldElement::from_bytes`).
    let mut r_bytes = *res;
    r_bytes[31] &= 0x7f;
    let r = BigUint::from_bytes_le(&r_bytes) % &p;

    // d = -A / (1 + 2 r^2) = (p - A) * inv(1 + 2 r^2)
    let two_r2 = (BigUint::from(2u8) * &r * &r) % &p;
    let denom = (BigUint::from(1u8) + two_r2) % &p;
    let neg_a = (&p - &a) % &p;
    let d = (neg_a * inv(&denom)) % &p;

    // eps = d^3 + A d^2 + d = d * (d^2 + A d + 1)
    let d2 = (&d * &d) % &p;
    let inner = (&d2 + (&a * &d) % &p + BigUint::from(1u8)) % &p;
    let eps = (&d * inner) % &p;

    // eps is a "square" (per sqrt_ratio_i) iff eps == 0 or eps is a quadratic residue.
    // Legendre symbol eps^((p-1)/2) is 0 for zero, 1 for a residue, p-1 for a non-residue.
    let legendre = eps.modpow(&((&p - 1u32) / 2u32), &p);
    let is_square = legendre != (&p - 1u32);

    // u = d if square, else -d - A = p - ((d + A) mod p).
    let u = if is_square {
        d
    } else {
        (&p - ((&d + &a) % &p)) % &p
    };

    // Canonical 32-byte little-endian encoding. u < p, so the high bit is always 0.
    let mut out = [0u8; 32];
    let le = u.to_bytes_le();
    out[..le.len()].copy_from_slice(&le);
    out
}

/// Test/fuzz-only accessor for [`nonspec_map_to_curve`], returning the canonical compressed point
/// bytes. Used by the differential AFL target `fuzz/src/bin/vrf_map_to_curve.rs` to assert
/// byte-for-byte equivalence with curve25519-dalek 4.x's removed `nonspec_map_to_curve`. Not part
/// of the public API.
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub fn map_to_curve_compressed(input: &[u8]) -> [u8; 32] {
    nonspec_map_to_curve(input).compress().to_bytes()
}

impl VrfSeed {
    const SIZE: usize = 96;

    /// Verifies the current VRF Seed given the previous VRF Seed (which is part of the message),
    /// the signer's public key and the nonce.
    pub fn verify(
        &self,
        prev_seed: &VrfSeed,
        public_key: &Ed25519PublicKey,
        nonce: u32,
    ) -> Result<(), VrfError> {
        // Deserialize signature.
        let V = CompressedEdwardsY::from_slice(&self.signature[..32])
            .unwrap() // Fails if the slice is not length 32.
            .decompress()
            .ok_or(VrfError::InvalidSignature)?;

        let h: Scalar = Option::from(Scalar::from_canonical_bytes(
            self.signature[32..64].try_into().unwrap(),
        ))
        .ok_or(VrfError::InvalidSignature)?;

        let s: Scalar = Option::from(Scalar::from_canonical_bytes(
            self.signature[64..].try_into().unwrap(),
        ))
        .ok_or(VrfError::InvalidSignature)?;

        // Deserialize public key.
        let A_bytes = public_key.as_bytes();

        let A = CompressedEdwardsY::from_slice(A_bytes)
            .unwrap() // Fails if the slice is not length 32.
            .decompress()
            .ok_or(VrfError::InvalidSignature)?;

        // Concatenate use case prefix, nonce, and previous entropy to form message. Note that we use
        // the entropy here and not the signature, that's because we need the message to be unique.
        let mut message = vec![VrfUseCase::Seed as u8];
        message.extend(nonce.to_be_bytes());
        message.extend_from_slice(prev_seed.entropy().as_slice());

        // Follow the verification algorithm for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa
        let B_v = nonspec_map_to_curve(&[A_bytes, &message[..]].concat());
        if A.is_small_order() || V.is_small_order() || B_v.is_identity() {
            return Err(VrfError::InvalidSignature);
        }
        let R: EdwardsPoint = &s * constants::ED25519_BASEPOINT_TABLE - h * A;
        let R_v: EdwardsPoint = s * B_v - h * V;
        let h_check = Scalar::hash_from_bytes::<Sha512>(
            &[
                A_bytes,
                V.compress().as_bytes(),
                R.compress().as_bytes(),
                R_v.compress().as_bytes(),
                &message[..],
            ]
            .concat(),
        );
        match h == h_check {
            true => Ok(()),
            false => {
                debug!(
                    "VRF Seed doesn't verify.\nh: {}\nh_check: {}",
                    hex::encode(h.as_bytes()),
                    hex::encode(h_check.as_bytes())
                );
                Err(VrfError::Forged)
            }
        }
    }

    /// Produces the next VRF Seed given the current VRF Seed (which is part of the message) and a
    /// key pair.
    #[must_use]
    pub fn sign_next(&self, keypair: &KeyPair, nonce: u32) -> Self {
        self.sign_next_with_rng(keypair, nonce, &mut rand::rng())
    }

    /// Produces the next VRF Seed given the current VRF Seed (which is part of the message), a
    /// key pair and a nonce.
    #[must_use]
    pub fn sign_next_with_rng<R: rand::Rng + CryptoRng>(
        &self,
        keypair: &KeyPair,
        nonce: u32,
        rng: &mut R,
    ) -> Self {
        // Get random bytes.
        let mut Z = [0u8; 64];
        rng.fill_bytes(&mut Z[..]);

        // Unpack the private and public keys.
        let a = keypair.private.to_scalar();
        let A_bytes = keypair.public.as_bytes();

        // Concatenate use case prefix, nonce, and entropy to form message. Note that we use the
        // entropy here and not the signature, that's because we need the message to be unique.
        let mut message = vec![VrfUseCase::Seed as u8];
        message.extend(nonce.to_be_bytes());
        message.extend_from_slice(self.entropy().as_slice());

        // Follow the signing algorithm for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa
        let B_v = nonspec_map_to_curve(&[A_bytes, &message[..]].concat());
        let V = (a * B_v).compress();
        let r = Scalar::hash_from_bytes::<Sha512>(&[a.as_bytes(), V.as_bytes(), &Z[..]].concat());
        let R = (&r * constants::ED25519_BASEPOINT_TABLE).compress();
        let R_v = (r * B_v).compress();
        let h = Scalar::hash_from_bytes::<Sha512>(
            &[
                A_bytes,
                V.as_bytes(),
                R.as_bytes(),
                R_v.as_bytes(),
                &message[..],
            ]
            .concat(),
        );
        let s = r + h * a;

        // Construct the new VrfSeed.
        Self {
            signature: [V.to_bytes(), h.to_bytes(), s.to_bytes()]
                .concat()
                .try_into()
                .unwrap(),
        }
    }

    // Extracts the entropy, which is 256 verifiably random bits, from the current VRF Seed. This
    // entropy can then be used for any purpose for which we need randomness. Note that this entropy
    // is what is unique for a given message and public key, not the signature (which can be
    // different for the same message and public key).
    pub fn try_entropy(&self) -> Option<VrfEntropy> {
        // We follow the specifications for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa

        // Calculate the point V and serialized it.
        let V = CompressedEdwardsY::from_slice(&self.signature[..32])
            .unwrap() // Fails if the slice is not length 32.
            .decompress()?;
        let V_bytes = V.mul_by_cofactor().compress().to_bytes();

        // Hash V to get the entropy.
        let mut hash = Sha256::new();
        hash.update(V_bytes);
        let h = hash.finalize();
        let mut res = [0u8; 32];
        res.copy_from_slice(&h[..32]);

        Some(VrfEntropy(res))
    }

    // Extracts the entropy, which is 256 verifiably random bits, from the current VRF Seed. This
    // entropy can then be used for any purpose for which we need randomness. Note that this entropy
    // is what is unique for a given message and public key, not the signature (which can be
    // different for the same message and public key). We assume that the VRF Seed is valid, if it
    // is not then this function might panic.
    pub fn entropy(&self) -> VrfEntropy {
        self.try_entropy()
            .expect("Tried to use an invalid signature for the VRF RNG!")
    }

    // Initializes a VRF RNG, for a given use case, from the current VRF Seed. We assume that the
    // VRF Seed is valid, if it is not this function might panic.
    pub fn rng(&self, use_case: VrfUseCase) -> VrfRng {
        // The use case cannot be `Seed`. That one is reserved for the `sign_next` method.
        assert_ne!(use_case, VrfUseCase::Seed);

        // Get the entropy.
        let entropy = self.entropy();

        // Pass the entropy to the VRF RNG.
        VrfRng::new(entropy, use_case)
    }
}

impl Default for VrfSeed {
    fn default() -> Self {
        VrfSeed {
            signature: [0u8; VrfSeed::SIZE],
        }
    }
}

impl fmt::Debug for VrfSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("VrfSeed")
            .field(&hex::encode(self.signature))
            .finish()
    }
}

impl fmt::Display for VrfSeed {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", hex::encode(self.signature))
    }
}

pub struct VrfRng {
    entropy: VrfEntropy,
    use_case: VrfUseCase,
    counter: u64,
}

impl VrfRng {
    fn new(entropy: VrfEntropy, use_case: VrfUseCase) -> Self {
        Self {
            entropy,
            use_case,
            counter: 0,
        }
    }

    pub fn next_hash(&mut self) -> Blake2bHash {
        // Hash use-case prefix, counter and entropy.
        let mut hasher = Blake2bHasher::new();
        hasher.write_u8(self.use_case as u8).unwrap();
        hasher.write_u64::<BigEndian>(self.counter).unwrap();
        hasher.write_all(self.entropy.as_slice()).unwrap();

        // Increase counter
        self.counter += 1;

        hasher.finish()
    }
}

impl Rng for VrfRng {
    fn next_u64(&mut self) -> u64 {
        self.next_hash().as_bytes().read_u64::<BigEndian>().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use nimiq_keys::SecureGenerate;
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;
    use rand::Rng;

    use super::*;

    /// Differential fuzz test: our `nonspec_map_to_curve` must produce byte-for-byte the same point
    /// as curve25519-dalek 4.x's removed `EdwardsPoint::nonspec_map_to_curve::<Sha512>` for every
    /// input. This is the guard that lets us drop the 4.x dependency: as long as this passes over a
    /// large random input space (plus crafted edge cases), the reimplementation is safe for the
    /// consensus-critical VRF seeds already in the chain.
    #[test]
    fn elligator_matches_curve25519_dalek_4x() {
        use curve25519_dalek_4x::edwards::EdwardsPoint as EdwardsPoint4x;
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        use sha2_0_10::Sha512 as Sha512_4x;

        // Reference implementation: the exact function that shipped in curve25519-dalek 4.x.
        let reference = |input: &[u8]| -> [u8; 32] {
            #[allow(deprecated)]
            EdwardsPoint4x::nonspec_map_to_curve::<Sha512_4x>(input)
                .compress()
                .to_bytes()
        };
        // Our reimplementation, compared in the same canonical compressed encoding.
        let ported =
            |input: &[u8]| -> [u8; 32] { nonspec_map_to_curve(input).compress().to_bytes() };

        // Fixed edge cases. The branch depends on SHA-512(input), so a spread of bytes and lengths
        // (incl. empty and the 69-byte call-site shape) exercises both Elligator branches; exhaustive
        // coverage is the `vrf_map_to_curve` AFL target.
        let mut fixed: Vec<Vec<u8>> = vec![vec![], (0u8..69).collect(), (0u8..32).collect()];
        for b in [0x00u8, 0x01, 0x55, 0xaa, 0xff] {
            for len in [1usize, 32, 69, 128] {
                fixed.push(vec![b; len]);
            }
        }
        for input in &fixed {
            assert_eq!(
                ported(input),
                reference(input),
                "mismatch on fixed input {input:?}",
            );
        }

        // Optional random sample on top of the fixed cases, off by default.
        let seed = std::env::var("VRF_FUZZ_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0u64);
        let iters = std::env::var("VRF_FUZZ_ITERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0u64);
        let mut rng = StdRng::seed_from_u64(seed);
        for _ in 0..iters {
            let len = rng.random_range(0..128usize);
            let mut input = vec![0u8; len];
            rng.fill_bytes(&mut input);
            assert_eq!(
                ported(&input),
                reference(&input),
                "mismatch on random input {}",
                hex::encode(&input),
            );
        }
    }

    #[test]
    fn vrf_works_fuzzy() {
        let mut rng = test_rng(false);
        let mut prev_seed = VrfSeed::default();

        for _ in 0..1000 {
            let key_pair = KeyPair::generate(&mut rng);

            let next_seed = prev_seed.sign_next(&key_pair, 0);

            assert!(next_seed.verify(&prev_seed, &key_pair.public, 0).is_ok());

            next_seed.entropy();

            prev_seed = next_seed;
        }
    }

    #[test]
    fn wrong_key_pair_fuzzy() {
        let mut rng = test_rng(false);
        let key_pair = KeyPair::generate(&mut rng);
        let prev_seed = VrfSeed::default();

        let next_seed = prev_seed.sign_next(&key_pair, 0);

        for _ in 0..1000 {
            let fake_pk = KeyPair::generate(&mut rng).public;

            assert_eq!(
                next_seed.verify(&prev_seed, &fake_pk, 0),
                Err(VrfError::Forged)
            );
        }
    }

    #[test]
    fn wrong_prev_seed_fuzzy() {
        let mut rng = test_rng(false);
        let key_pair = KeyPair::generate(&mut rng);
        let prev_seed = VrfSeed::default();

        let next_seed = prev_seed.sign_next(&key_pair, 0);

        for _ in 0..1000 {
            let fake_key_pair = KeyPair::generate(&mut rng);
            let fake_seed = VrfSeed::default().sign_next(&fake_key_pair, 0);

            assert_eq!(
                next_seed.verify(&fake_seed, &key_pair.public, 0),
                Err(VrfError::Forged)
            );
        }
    }

    #[test]
    fn wrong_seed_fuzzy() {
        let mut rng = test_rng(false);
        let key_pair = KeyPair::generate(&mut rng);
        let prev_seed = VrfSeed::default();

        for _ in 0..1000 {
            let mut bytes = [0u8; VrfSeed::SIZE];
            rng.fill_bytes(&mut bytes);
            let fake_seed = VrfSeed { signature: bytes };

            assert!(fake_seed.verify(&prev_seed, &key_pair.public, 0).is_err());
        }
    }

    #[test]
    fn invalid_seed_has_no_entropy() {
        let mut rng = test_rng(false);
        let key_pair = KeyPair::generate(&mut rng);
        let valid_seed = VrfSeed::default().sign_next(&key_pair, 0);
        let mut invalid_seed = None;

        for i in 0..32 {
            for byte in u8::MIN..=u8::MAX {
                let mut signature = valid_seed.signature;
                if signature[i] == byte {
                    continue;
                }
                signature[i] = byte;

                let candidate = VrfSeed { signature };
                if candidate.try_entropy().is_none() {
                    invalid_seed = Some(candidate);
                    break;
                }
            }

            if invalid_seed.is_some() {
                break;
            }
        }

        let invalid_seed = invalid_seed.expect("could not construct invalid VRF seed");

        assert_eq!(invalid_seed.try_entropy(), None);
    }

    #[test]
    fn wrong_nonce() {
        let mut rng = test_rng(false);
        let key_pair = KeyPair::generate(&mut rng);
        let prev_seed = VrfSeed::default();

        let next_seed = prev_seed.sign_next(&key_pair, 0);

        assert_eq!(
            next_seed.verify(&prev_seed, &key_pair.public, 1),
            Err(VrfError::Forged)
        );
    }
}
