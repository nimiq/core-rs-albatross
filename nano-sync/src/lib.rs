#![allow(dead_code)]

use std::io;

use ark_mnt6_753::G2Projective;
use ark_relations::r1cs::SynthesisError;
use ark_serialize::SerializationError;
use constants::{PK_TREE_BREADTH, VALIDATOR_SLOTS};
use thiserror::Error;
use nimiq_nano_primitives::pk_tree_construct as pk_t_c;

// Re-export big-endian serialization of algebra types.
pub use nimiq_bls::compression;
// Re-export randomness generation.
pub use nimiq_bls::rand_gen;

pub mod circuits;
pub mod constants;
pub mod cost_analysis;
pub mod gadgets;
pub mod primitives;
mod prove;
mod setup;
mod verify;

/// This the main struct for the nano-sync crate. It provides methods to setup (create the
/// proving and verifying keys), create proofs and verify proofs for the nano sync circuit.
pub struct NanoZKP;

#[derive(Error, Debug)]
pub enum NanoZKPError {
    #[error("filesystem error")]
    Filesystem(#[from] io::Error),
    #[error("serialization error")]
    Serialization(#[from] SerializationError),
    #[error("circuit error")]
    Circuit(#[from] SynthesisError),
}

#[inline]
pub fn pk_tree_construct(public_keys: Vec<G2Projective>) -> Vec<u8> {
    pk_t_c(public_keys, VALIDATOR_SLOTS, PK_TREE_BREADTH)
}
