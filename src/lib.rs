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
// TODO: Change all the vks to constants.
// TODO: Improve the PKTree circuit to not need Merkle trees for the agg pks.
// Note: ~5m proving time per 1M constraints. 10GB memory per 1M constraints.
