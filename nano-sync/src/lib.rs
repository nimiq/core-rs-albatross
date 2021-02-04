#![allow(dead_code)]

use std::io;

use ark_relations::r1cs::SynthesisError;
use ark_serialize::SerializationError;
use thiserror::Error;

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
pub mod utils;
mod verify;

#[derive(Error, Debug)]
pub enum NanoZKPError {
    #[error("filesystem error")]
    Filesystem(#[from] io::Error),
    #[error("serialization error")]
    Serialization(#[from] SerializationError),
    #[error("circuit error")]
    Circuit(#[from] SynthesisError),
}

pub struct NanoZKP;
