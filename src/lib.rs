#![allow(dead_code)]

pub mod circuits;
pub mod compression;
pub mod constants;
pub mod cost_analysis;
pub mod gadgets;
pub mod primitives;
pub mod rand_gen;
pub mod utils;

// TODO: Create the specific instances of the wrapper and merger circuits that we need.
// - You separate the wrapper into two new circuits: macro block wrapper and merger wrapper.
// - The merger and the merger wrapper receive both the vks_mnt6_hash and the vks_mnt4_hash as inputs.
// - The macro block wrapper receives the vks_mnt4_hash as input.
// - The macro block receives neither.
// - The merger wrapper (or maybe the merger itself?) must have a special condition that makes the proof
//   pass if both input state hashes are equal. This is necessary for the genesis block. The chain needs
//   to start somewhere.
// TODO: Fix the y_to_bit_g2 gadget.
// TODO: Optimize the state commitments (and the vks commitments). Right now it won't work with 1024 pks.
// - final pks don't need to be serialized in-circuit. (I only need the commitment to it)
// - recursive snarks tree for the initial pks.
// TODO: Finish the examples.
// Note: ~5m per 1M constraints proving time. 10GB memory per 1M constraints.
