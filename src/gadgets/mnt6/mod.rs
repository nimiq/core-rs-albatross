//! This module contains the zk-SNARK "gadgets" that are meant to be used with circuits on the
//! MNT6-753 curve. This means that they can manipulate elliptic curve points on the  MNT4-753 curve.

pub use pedersen::*;
pub use y_to_bit::*;

mod pedersen;
mod y_to_bit;
