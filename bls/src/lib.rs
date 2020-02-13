#![allow(dead_code)]

#[macro_use]
extern crate failure;
extern crate hex;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;

// Imports the types needed for elliptic curve algebra
use algebra::{
    biginteger::BigInteger,
    bytes::{FromBytes, ToBytes},
    curves::{
        bls12_377::{Bls12_377, G1Affine, G1Projective, G2Affine, G2Projective},
        AffineCurve, PairingEngine, ProjectiveCurve,
    },
    fields::{
        bls12_377::{Fq, Fr},
        PrimeField,
    },
    rand::UniformRand,
    serialize::{CanonicalDeserialize, CanonicalSerialize, SerializationError},
};
use num_traits::{One, Zero};

// Used for the Blake2X hashing.
use crypto_primitives::prf::Blake2sWithParameterBlock;

// Used for the random number generation
use utils::key_rng::{CryptoRng, Rng};
pub use utils::key_rng::{SecureGenerate, SecureRng};

#[cfg(feature = "beserial")]
use beserial::{Deserialize, Serialize};
use blake2_rfc::blake2s::Blake2s;
use failure::Fail;
use hash::{Blake2sHash, Hash};
use hex::FromHexError;
use std::{cmp::Ordering, fmt, str::FromStr};

// Implements several serialization-related types
#[cfg(feature = "beserial")]
pub mod serialization;

// Implements the LazyPublicKey type. Which is a faster, cached version of PublicKey.
#[cfg(feature = "lazy")]
pub mod lazy;

// Implements all of the types needed to do BLS signatures.
mod types;
pub use types::*;

// Specifies the hash algorithm used for signatures
pub type SigHash = Blake2sHash;

/// If bytes is a little endian representation of a number, this would return the bits of the
/// number in descending order
pub fn bytes_to_bits(bytes: &[u8], bits_to_take: usize) -> Vec<bool> {
    let mut bits = vec![];
    for i in 0..bytes.len() {
        let mut byte = bytes[i];
        for _ in 0..8 {
            bits.push((byte & 1) == 1);
            byte >>= 1;
        }
    }

    let bits_filtered = bits
        .into_iter()
        .take(bits_to_take)
        .collect::<Vec<bool>>()
        .into_iter()
        .rev()
        .collect();

    bits_filtered
}
