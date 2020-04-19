//! This module contains the zk-SNARK circuits that use the MNT6-753 curve. This means that they
//! can manipulate elliptic curve points on the  MNT4-753 curve.

pub use macro_block_wrapper::MacroBlockWrapperCircuit;
pub use merger_wrapper::MergerWrapperCircuit;

pub mod macro_block_wrapper;
pub mod merger_wrapper;
