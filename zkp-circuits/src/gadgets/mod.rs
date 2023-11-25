//! This module contains all of the "gadgets" for the zk-SNARK circuits. These are smaller, modular pieces
//! of on-circuit code intended to facilitate the creation of larger circuits.

pub mod mnt4;
pub mod mnt6;

pub mod be_bytes;
pub mod bits;
pub mod compressed_affine;
pub mod compressed_vk;
pub mod ext_traits;
pub mod pedersen;
pub mod recursive_input;
pub mod serialize;
pub mod vk_commitment;
pub mod vks_commitment;
pub mod y_to_bit;
