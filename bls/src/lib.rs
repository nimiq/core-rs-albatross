#![allow(dead_code)]

#[macro_use]
extern crate failure;
extern crate hex;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;

use std::io::Error;
use std::{cmp::Ordering, fmt, ops::MulAssign, str::FromStr};

use algebra::bls12_377::{Bls12_377, Fq, Fq2, Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use algebra::short_weierstrass_jacobian::GroupAffine;
use algebra::{BigInteger384, SerializationError};
use algebra_core::bytes::{FromBytes, ToBytes};
use algebra_core::curves::{
    models::SWModelParameters, AffineCurve, PairingEngine, ProjectiveCurve,
};
use algebra_core::fields::PrimeField;
use algebra_core::UniformRand;
use blake2_rfc::blake2s::Blake2s;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use crypto_primitives::prf::Blake2sWithParameterBlock;
use failure::Fail;
use hex::FromHexError;
use log::error;
use num_traits::{One, Zero};

#[cfg(feature = "beserial")]
use beserial::{Deserialize, Serialize};
use hash::{Blake2sHash, Hash};
pub use types::*;
use utils::key_rng::{CryptoRng, Rng};
pub use utils::key_rng::{SecureGenerate, SecureRng};

// Implements big-endian compression
pub mod compression;

// Implements several serialization-related types
#[cfg(feature = "beserial")]
pub mod serialization;

// Implements the LazyPublicKey type. Which is a faster, cached version of PublicKey.
#[cfg(feature = "lazy")]
pub mod lazy;

// Implements all of the types needed to do BLS signatures.
mod types;

// Specifies the hash algorithm used for signatures
pub type SigHash = Blake2sHash;

pub fn big_int_from_bytes_be<R: std::io::Read>(reader: &mut R) -> BigInteger384 {
    let mut res = [0u64; 6];
    for num in res.iter_mut().rev() {
        let mut bytes = [0u8; 8];
        reader.read_exact(&mut bytes).unwrap();
        *num = u64::from_be_bytes(bytes);
    }
    BigInteger384::new(res)
}
