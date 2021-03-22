#![allow(dead_code)]

use std::io;

use ark_groth16::Proof;
use ark_mnt6_753::G2Projective;
use ark_mnt6_753::MNT6_753;
use ark_relations::r1cs::SynthesisError;
use ark_serialize::SerializationError;
use thiserror::Error;

use constants::{PK_TREE_BREADTH, VALIDATOR_SLOTS};
use nimiq_nano_primitives::pk_tree_construct as pk_t_c;

#[cfg(feature = "prover")]
pub(crate) mod circuits;
#[cfg(feature = "prover")]
pub(crate) mod gadgets;
#[cfg(feature = "prover")]
pub(crate) mod prove;
#[cfg(feature = "prover")]
pub(crate) mod setup;

pub mod constants;
pub mod primitives;
pub mod utils;
pub(crate) mod verify;

/// This the main struct for the nano-sync crate. It provides methods to setup (create the
/// proving and verifying keys), create proofs and verify proofs for the nano sync circuit.
pub struct NanoZKP;

/// This is the proof type for the NanoZKP. It is just an alias, for convenience.
pub type NanoProof = Proof<MNT6_753>;

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
