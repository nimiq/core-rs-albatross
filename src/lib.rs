#![allow(dead_code)]

pub use circuits::*;
pub use gadgets::*;
pub use primitives::*;

pub mod circuits;
pub mod constants;
pub mod cost_analysis;
pub mod gadgets;
pub mod primitives;
pub mod rand_gen;

// TODO: Integrate the MNT4/6 curves.
// TODO: Create the specific instances of the wrapper and merger circuits that we need.
// - You separate the wrapper into two new circuits: macro block wrapper and merger wrapper.
// - You create two new types of inputs, the vks_mnt6_hash and the vks_mnt4_hash. Both are Blake2s
//   hashes, similar to how state hashes work.
// - vks_mnt6_hash contains the verifying keys for the mnt6 circuits (both wrappers). This hash is
//   only opened in the mnt4 circuits.
// - vks_mnt4_hash contains the verifying keys for the mnt4 circuits (macro block and merger). This hash is
//   only opened in the mnt6 circuits.
// - The merger and the merger wrapper receive both the vks_mnt6_hash and the vks_mnt4_hash as inputs.
// - The macro block wrapper receives the vks_mnt4_hash as input.
// - The macro block receives neither.
// TODO: Finish the examples.
