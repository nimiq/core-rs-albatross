//! This module contains all of the "gadgets" for the zk-SNARK circuits. These are smaller, modular pieces
//! of on-circuit code intended to facilitate the creation of larger circuits.

pub use alloc_constant::*;

pub mod alloc_constant;
pub mod mnt4;
pub mod mnt6;
