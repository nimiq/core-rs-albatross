//! This module contains a series of cryptographic primitives that are off-circuit versions of the primitives
//! in the gadgets module. The main purpose of these primitives is to be used by other crates who might
//! need an off-circuit version of a primitive that is however guaranteed to be consistent with the on-circuit
//! primitive used by one of our zk-SNARKs. They are also used for testing.

pub use macro_block::*;
pub use merkle_tree::*;
// Re-export pedersen hashes from bls crate.
pub use nimiq_bls::pedersen::*;
pub use pk_tree::*;
pub use serialize::*;
pub use state_commitment::*;
pub use vk_commitment::*;

pub mod macro_block;
pub mod merkle_tree;
pub mod pk_tree;
pub mod serialize;
pub mod state_commitment;
pub mod vk_commitment;
