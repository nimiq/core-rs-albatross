#![allow(dead_code)]

pub mod circuits;
pub mod compression;
pub mod constants;
pub mod cost_analysis;
pub mod gadgets;
pub mod primitives;
pub mod rand_gen;
pub mod utils;

// TODO: Eliminate Macro Block Wrapper.
// TODO: Increase the PKTree to 32 leaves.
// TODO: Finish the examples.
// TODO: Change all the vks to constants.
// TODO: Redo tests. (Consider having the verifying gadgets return bool)
// Note: ~5m proving time per 1M constraints. 10GB memory per 1M constraints.
// Note: 819,200 max constraints in the MNT6. Merger circuit has 777,842. Macro Block (with 4 validators)
//       has 905,185.
