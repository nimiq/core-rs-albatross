//! This crate contains a series of cryptographic primitives that are "off-circuit" versions of the primitives
//! in the zkp crate. The main purpose of these primitives is to be used by other crates who might
//! need an off-circuit version of a primitive that is however guaranteed to be consistent with the on-circuit
//! primitive used by one of our zk-SNARKs. They are also used for testing.

pub use pedersen::pedersen_parameters_mnt6;
pub use serialize::*;
pub use vk_commitment::*;

pub mod ext_traits;
pub mod pedersen;
mod serialize;
mod vk_commitment;

use std::io;

use ark_relations::r1cs::SynthesisError;
use ark_serialize::SerializationError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NanoZKPError {
    #[error("filesystem error: {0}")]
    Filesystem(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] SerializationError),
    #[error("serialization error: {0}")]
    Serializing(#[from] nimiq_serde::DeserializeError),
    #[error("circuit error: {0}")]
    Circuit(#[from] SynthesisError),
    #[error("empty proof")]
    EmptyProof,
    #[error("invalid block")]
    InvalidBlock,
    #[error("invalid metadata")]
    InvalidMetadata,
}
