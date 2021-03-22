use std::io;

use ark_groth16::Proof;
use ark_mnt6_753::MNT6_753;
use ark_relations::r1cs::SynthesisError;
use ark_serialize::SerializationError;
use thiserror::Error;

#[cfg(feature = "prover")]
mod prove;
#[cfg(feature = "prover")]
mod setup;
mod verify;

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
