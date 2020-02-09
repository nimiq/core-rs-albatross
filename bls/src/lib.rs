#![allow(dead_code)]

#[macro_use]
extern crate failure;
extern crate hex;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;

// Imports the types needed for elliptic curve algebra
use algebra::{
    bytes::{FromBytes, ToBytes},
    curves::{
        bls12_377::{Bls12_377, G1Affine, G1Projective, G2Affine, G2Projective},
        AffineCurve, PairingEngine, ProjectiveCurve,
    },
    fields::bls12_377::{Fq, Fr},
    rand::UniformRand,
    serialize::{CanonicalDeserialize, CanonicalSerialize, SerializationError},
};

// Zero is for used for getting the point at infinity from a curve.
use num_traits::Zero;

// Used for the random number generation
use rand::SeedableRng;
use rand_chacha::ChaChaRng;
use utils::key_rng::{CryptoRng, Rng};
pub use utils::key_rng::{SecureGenerate, SecureRng};

#[cfg(feature = "beserial")]
use beserial::{Deserialize, Serialize};
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
