#![allow(dead_code)]

use nimiq_hash::Blake2sHash;
pub use types::*;

// Implements big-endian compression
pub mod compression;

// Implements several serialization-related types
#[cfg(feature = "beserial")]
pub mod serialization;

// Implements the LazyPublicKey type. Which is a faster, cached version of PublicKey.
#[cfg(feature = "lazy")]
pub mod lazy;

// Implements all of the types needed to do BLS signatures.
mod types;

// Specifies the hash algorithm used for signatures
pub type SigHash = Blake2sHash;
