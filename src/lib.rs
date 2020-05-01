#![allow(dead_code)]

pub mod circuits;
pub mod compression;
pub mod constants;
pub mod cost_analysis;
pub mod gadgets;
pub mod primitives;
pub mod rand_gen;
pub mod utils;

// TODO: Finish the examples.
// TODO: Redo tests. (Consider having the verifying gadgets return bool)
// TODO: Change all the vks to constants.
// Note: ~5m per 1M constraints proving time. 10GB memory per 1M constraints.
// Note: 819,200 max constraints in the MNT6. Merger circuit has 777,842. Macro Block (with 4 validators)
//       has 905,185.
