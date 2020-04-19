//! This module contains the zk-SNARK circuits that use the MNT4-753 curve. This means that they
//! can manipulate elliptic curve points on the  MNT6-753 curve.

pub use macro_block::MacroBlockCircuit;
pub use merger::MergerCircuit;

pub mod macro_block;
pub mod merger;
