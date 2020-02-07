// The projective form is the longer one, with 3 coordinates. It is meant only for quick calculation.
// We can't use the affine form since the Algebra library doesn't support arithmetic with it.

// Add some note about rogue key attacks and proofs of knowledge.
#![allow(dead_code)]

#[macro_use]
extern crate failure;
extern crate hex;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;

// imports main types needed for EC algebra
use algebra::{
    bytes::{FromBytes, ToBytes},
    curves::{
        bls12_377::{Bls12_377, G1Affine, G1Projective, G2Affine, G2Projective},
        AffineCurve, PairingEngine, ProjectiveCurve,
    },
    fields::{
        bls12_377::{Fq, Fq12, Fr},
        Field, FpParameters, PrimeField,
    },
    rand::UniformRand,
    serialize::{CanonicalDeserialize, CanonicalSerialize, SerializationError},
};

// Zero is for used for getting the point at infinity from a curve.
// One is used to get the identity element from a finite field.
use num_traits::{One, Zero};

// used for rng
use rand::SeedableRng;
use rand_chacha::ChaChaRng;
use utils::key_rng::{CryptoRng, Rng};
pub use utils::key_rng::{SecureGenerate, SecureRng};

use hash::{Blake2bHash, Hash};
use std::{cmp::Ordering, fmt, str::FromStr};

#[cfg(feature = "beserial")]
use beserial::{Deserialize, Serialize};

#[cfg(feature = "lazy")]
pub mod lazy;
use lazy::*;

use failure::Fail;
use hex::FromHexError;

#[cfg(feature = "beserial")]
pub mod serialization;
use serialization::*;

// Hash used for signatures
pub type SigHash = Blake2bHash;

mod types;
use types::*;

#[cfg(test)]
mod tests;
