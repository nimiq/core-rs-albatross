#![allow(non_snake_case)]

#[cfg(feature = "serde-derive")]
use std::borrow::Cow;
use std::{fmt, hash::Hash, io::Write};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use curve25519_dalek::{
    constants,
    edwards::{CompressedEdwardsY, EdwardsPoint},
    scalar::Scalar,
    traits::IsIdentity,
};
use log::debug;
use nimiq_hash::{Blake2bHash, Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Ed25519PublicKey, KeyPair};
#[cfg(feature = "serde-derive")]
use nimiq_macros::add_serialization_fns_typed_arr;
use nimiq_macros::create_typed_array;
use rand::{CryptoRng, RngCore};
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

impl std::fmt::Debug for VrfEntropy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("VrfEntropy")
            .field(&hex::encode(self.0))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
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

impl VrfSeed {
    pub const SIZE: usize = 96;

    /// Verifies the current VRF Seed given the previous VRF Seed (which is part of the message)
    /// and the signer's public key.
    pub fn verify(
        &self,
        prev_seed: &VrfSeed,
        public_key: &Ed25519PublicKey,
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

        // Concatenate use case prefix and previous entropy to form message. Note that we use the
        // entropy here and not the signature, that's because we need the message to be unique.
        let mut message = vec![VrfUseCase::Seed as u8];
        message.extend_from_slice(prev_seed.entropy().as_slice());

        // Follow the verification algorithm for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa
        #[allow(deprecated)]
        let B_v = EdwardsPoint::nonspec_map_to_curve::<Sha512>(&[A_bytes, &message[..]].concat());
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
    pub fn sign_next(&self, keypair: &KeyPair) -> Self {
        self.sign_next_with_rng(keypair, &mut rand::thread_rng())
    }

    /// Produces the next VRF Seed given the current VRF Seed (which is part of the message) and a
    /// key pair.
    #[must_use]
    pub fn sign_next_with_rng<R: RngCore + CryptoRng>(
        &self,
        keypair: &KeyPair,
        rng: &mut R,
    ) -> Self {
        // Get random bytes.
        let mut Z = [0u8; 64];
        rng.fill_bytes(&mut Z[..]);

        // Unpack the private and public keys.
        let a = keypair.private.to_scalar();
        let A_bytes = keypair.public.as_bytes();

        // Concatenate use case prefix and entropy to form message. Note that we use the entropy
        // here and not the signature, that's because we need the message to be unique.
        let mut message = vec![VrfUseCase::Seed as u8];
        message.extend_from_slice(self.entropy().as_slice());

        // Follow the signing algorithm for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa
        #[allow(deprecated)]
        let B_v = EdwardsPoint::nonspec_map_to_curve::<Sha512>(&[A_bytes, &message[..]].concat());
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
    // different for the same message and public key). We assume that the VRF Seed is valid, if it
    // is not then this function might panic.
    pub fn entropy(&self) -> VrfEntropy {
        // We follow the specifications for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa

        // Calculate the point V and serialized it.
        let V = CompressedEdwardsY::from_slice(&self.signature[..32])
            .unwrap() // Fails if the slice is not length 32.
            .decompress()
            .expect("Tried to use an invalid signature for the VRF RNG!");
        let V_bytes = V.mul_by_cofactor().compress().to_bytes();

        // Hash V to get the entropy.
        let mut hash = Sha256::new();
        hash.update(V_bytes);
        let h = hash.finalize();
        let mut res = [0u8; 32];
        res.copy_from_slice(&h[..32]);

        VrfEntropy(res)
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
    use nimiq_test_utils::test_rng::test_rng;

    use super::*;

    #[test]
    fn vrf_works_fuzzy() {
        let mut rng = test_rng(false);
        let mut prev_seed = VrfSeed::default();

        for _ in 0..1000 {
            let key_pair = KeyPair::generate(&mut rng);

            let next_seed = prev_seed.sign_next(&key_pair);

            assert!(next_seed.verify(&prev_seed, &key_pair.public).is_ok());

            next_seed.entropy();

            prev_seed = next_seed;
        }
    }

    #[test]
    fn wrong_key_pair_fuzzy() {
        let mut rng = test_rng(false);
        let key_pair = KeyPair::generate(&mut rng);
        let prev_seed = VrfSeed::default();

        let next_seed = prev_seed.sign_next(&key_pair);

        for _ in 0..1000 {
            let fake_pk = KeyPair::generate(&mut rng).public;

            assert_eq!(
                next_seed.verify(&prev_seed, &fake_pk),
                Err(VrfError::Forged)
            );
        }
    }

    #[test]
    fn wrong_prev_seed_fuzzy() {
        let mut rng = test_rng(false);
        let key_pair = KeyPair::generate(&mut rng);
        let prev_seed = VrfSeed::default();

        let next_seed = prev_seed.sign_next(&key_pair);

        for _ in 0..1000 {
            let fake_key_pair = KeyPair::generate(&mut rng);
            let fake_seed = VrfSeed::default().sign_next(&fake_key_pair);

            assert_eq!(
                next_seed.verify(&fake_seed, &key_pair.public),
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

            assert!(fake_seed.verify(&prev_seed, &key_pair.public).is_err());
        }
    }
}
