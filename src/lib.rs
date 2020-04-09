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
// - Wrapper for the macro block gets the vk for the macro block circuit as constant.
// - Merger gets the vk for the macro block wrapper as constant.
// - Both merger and wrapper for the merger get two extra inputs, two verifying keys.
// - The two verifying keys are the ones for the merger and wrapper for the merger.
// - That means both merger and wrapper for the merger have the exact same public inputs.
// TODO: Finish the examples.
