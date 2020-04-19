#![allow(dead_code)]

pub mod circuits;
pub mod compression;
pub mod constants;
pub mod cost_analysis;
pub mod gadgets;
pub mod primitives;
pub mod rand_gen;
pub mod utils;

// TODO: Fix the YToBit gadget. (+ alloc_const in zexe)
// TODO: Optimize the state commitments. Right now it won't work with 1024 pks.
// - final pks don't need to be serialized in-circuit. (I only need the commitment to it)
// - recursive snarks tree for the initial pks. (2^4)
// TODO: Finish the examples.
// TODO: Redo tests.
// Note: ~5m per 1M constraints proving time. 10GB memory per 1M constraints.
