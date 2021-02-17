#![allow(dead_code)]
use std::io;

use ark_mnt6_753::G2Projective;
use ark_relations::r1cs::SynthesisError;
use ark_serialize::SerializationError;
use constants::{PK_TREE_BREADTH, VALIDATOR_SLOTS};
use nimiq_nano_primitives::pk_tree_construct as pk_t_c;
use thiserror::Error;

#[cfg(feature = "prover")]
pub(crate) mod circuits;
#[cfg(feature = "prover")]
pub(crate) mod gadgets;
#[cfg(feature = "prover")]
pub(crate) mod prove;
#[cfg(feature = "prover")]
pub(crate) mod setup;

#[cfg(any(feature = "verifier", feature = "prover"))]
pub(crate) mod verify;

pub mod constants;
pub mod primitives;
pub mod utils;

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
