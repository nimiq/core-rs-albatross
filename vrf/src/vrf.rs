#![allow(non_snake_case)]

use std::fmt;
use std::hash::Hash;
use std::io::Write;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use curve25519_dalek::constants;
use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::IsIdentity;
use rand::prelude::*;
#[cfg(feature = "serde-derive")]
use serde_big_array::big_array;
use sha2::{Digest, Sha256, Sha512};

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_hash::{Blake2bHash, Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{KeyPair, PublicKey};

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

#[cfg(feature = "serde-derive")]
big_array! { BigArray; }

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
/// A struct containing a VRF Seed. It simply the serialized output of the VXEdDSA algorithm.
/// https://www.signal.org/docs/specifications/xeddsa/#vxeddsa
pub struct VrfSeed {
    #[cfg_attr(feature = "serde-derive", serde(with = "BigArray"))]
    signature: [u8; 96],
}

impl VrfSeed {
    /// Verifies the current VRF Seed given the previous VRF Seed (which is part of the message)
    /// and the signer's public key.
    pub fn verify(&self, prev_seed: &VrfSeed, public_key: &PublicKey) -> Result<(), VrfError> {
        // Deserialize signature.
        let V = CompressedEdwardsY::from_slice(&self.signature[..32])
            .decompress()
            .ok_or(VrfError::InvalidSignature)?;

        let h = Scalar::from_canonical_bytes(self.signature[32..64].try_into().unwrap())
            .ok_or(VrfError::InvalidSignature)?;

        let s = Scalar::from_canonical_bytes(self.signature[64..].try_into().unwrap())
            .ok_or(VrfError::InvalidSignature)?;

        // Deserialize public key.
        let A_bytes = public_key.as_bytes();

        let A = CompressedEdwardsY::from_slice(A_bytes)
            .decompress()
            .ok_or(VrfError::InvalidSignature)?;

        // Concatenate use case prefix and previous signature to form message.
        let mut message = vec![];
        message.push(VrfUseCase::Seed as u8);
        message.extend_from_slice(prev_seed.signature.as_ref());

        // Follow the verification algorithm for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa
        let B_v = EdwardsPoint::hash_from_bytes::<Sha512>(&[A_bytes, &message[..]].concat());
        if A.is_small_order() || V.is_small_order() || B_v.is_identity() {
            return Err(VrfError::InvalidSignature);
        }
        let R = &s * &constants::ED25519_BASEPOINT_TABLE - &h * &A;
        let R_v = &s * &B_v - &h * &V;
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
            false => Err(VrfError::Forged),
        }
    }

    /// Produces the next VRF Seed given the current VRF Seed (which is part of the message) and a
    /// key pair.
    pub fn sign_next(&self, keypair: &KeyPair) -> Self {
        // Get random bytes.
        let mut rng = rand::thread_rng();
        let mut Z = [0u8; 64];
        rng.fill_bytes(&mut Z[..]);

        // Unpack the private and public keys.
        let a = keypair.private.0.s;
        let A_bytes = keypair.public.as_bytes();

        // Concatenate use case prefix and signature to form message.
        let mut message = vec![];
        message.push(VrfUseCase::Seed as u8);
        message.extend_from_slice(self.signature.as_ref());

        // Follow the signing algorithm for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa
        let B_v = EdwardsPoint::hash_from_bytes::<Sha512>(&[A_bytes, &message[..]].concat());
        let V = (&a * &B_v).compress();
        let r = Scalar::hash_from_bytes::<Sha512>(&[a.as_bytes(), V.as_bytes(), &Z[..]].concat());
        let R = (&r * &constants::ED25519_BASEPOINT_TABLE).compress();
        let R_v = (&r * &B_v).compress();
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

    // Initializes a VRF RNG, for a given use case, from the current VRF Seed. We assume that the
    // VRF Seed is valid, if it is not this function might panic.
    pub fn rng(&self, use_case: VrfUseCase) -> VrfRng {
        // We follow the specifications for VXEdDSA.
        // https://www.signal.org/docs/specifications/xeddsa/#vxeddsa

        // Calculate the point V and serialized it.
        let V = CompressedEdwardsY::from_slice(&self.signature[..32])
            .decompress()
            .expect("Tried to use an invalid signature for the VRF RNG!");
        let V_bytes = V.mul_by_cofactor().compress().to_bytes();

        // Hash V to get the entropy.
        let mut hash = Sha256::new();
        hash.update(V_bytes);
        let h = hash.finalize();
        let mut res = [0u8; 32];
        res.copy_from_slice(&h[..32]);

        // Pass the entropy to the VRF RNG.
        VrfRng::new(res, use_case)
    }
}

impl Default for VrfSeed {
    fn default() -> Self {
        VrfSeed {
            signature: [0u8; 96],
        }
    }
}

impl fmt::Display for VrfSeed {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{:?}", self.signature)
    }
}

impl Serialize for VrfSeed {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write(&self.signature)?;
        Ok(96)
    }

    fn serialized_size(&self) -> usize {
        96
    }
}

impl Deserialize for VrfSeed {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0; 96];
        reader.read_exact(&mut bytes[..96])?;
        Ok(VrfSeed { signature: bytes })
    }
}

pub struct VrfRng {
    entropy: [u8; 32],
    use_case: VrfUseCase,
    counter: u64,
}

impl VrfRng {
    fn new(entropy: [u8; 32], use_case: VrfUseCase) -> Self {
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
        hasher.write_all(&self.entropy).unwrap();

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
