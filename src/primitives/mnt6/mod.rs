//! This module contains the cryptographic primitives that are meant to be used with circuits on the
//! MNT6-753 curve. This means that they can manipulate elliptic curve points on the  MNT4-753 curve.

pub use pedersen::*;
pub use vk_commitment::*;

pub mod pedersen;
mod vk_commitment;
