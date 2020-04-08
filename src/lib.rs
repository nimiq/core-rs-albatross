#![allow(dead_code)]

pub use circuits::*;
pub use gadgets::*;
pub use input::Input;
pub use primitives::*;

pub mod circuits;
pub mod constants;
pub mod cost_analysis;
pub mod gadgets;
pub mod input;
pub mod primitives;
pub mod rand_gen;

// TODO: Optimize the inputs by having the state hashes being Field numbers instead of 32 u8's.
// TODO: Finish the examples.
// TODO: Integrate the MNT4/6 curves.
// TODO: Create the specific instances of the wrapper and merger circuits that we need.
