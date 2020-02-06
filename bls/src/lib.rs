// The projective form is the longer one, with 3 coordinates. It is meant only for quick calculation.
// We can't use the affine form since the Algebra library doesn't support arithmetic with it.

// Add some note about rogue key attacks and proofs of knowledge.

#[macro_use]
extern crate failure;
extern crate hex;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;

// imports main types needed for EC algebra
use algebra::{
    curves::{
        bls12_377::{Bls12_377, G1Affine, G1Projective, G2Affine, G2Projective},
        AffineCurve, PairingEngine, ProjectiveCurve,
    },
    fields::{
        bls12_377::{Fq, Fq12, Fr},
        Field, FpParameters,
    },
    rand::UniformRand,
    CanonicalSerialize,
};

// Zero is for used for getting the point at infinity from a curve.
// One is used to get the identity element from a finite field.
use num_traits::{One, Zero};

use hashbrown::HashSet;
use rand::SeedableRng;
use rand_chacha::ChaChaRng;

use hash::{Blake2bHash, Hash};
use utils::key_rng::{CryptoRng, Rng};
pub use utils::key_rng::{SecureGenerate, SecureRng};

// pub mod bls12_381;
// #[cfg(feature = "beserial")]
// pub mod serialization;

/// Hash used for signatures
pub type SigHash = Blake2bHash;

mod aggregate_public_keys;
mod aggregate_signatures;
mod keypairs;
mod public_keys;
mod secret_keys;
mod signatures;

use aggregate_public_keys::*;
use aggregate_signatures::*;
use keypairs::*;
use public_keys::*;
use secret_keys::*;
use signatures::*;

#[cfg(test)]
mod tests;
