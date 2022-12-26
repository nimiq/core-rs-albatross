use nimiq_hash::Blake2sHash;
pub use types::*;

// Implements several serialization-related types.
#[cfg(feature = "beserial")]
pub mod serialization;

// Implements big-endian serialization of algebra types.
pub mod compression;

// Implements the LazyPublicKey type. Which is a faster, cached version of PublicKey.
#[cfg(feature = "lazy")]
pub mod lazy;

// Implements all of the types needed to do BLS signatures.
mod types;

// Utility functions.
pub mod utils;

// Pedersen hash functions.
pub mod pedersen;

// Randomness generation.
pub mod rand_gen;

// Specifies the hash algorithm used for signatures
pub type SigHash = Blake2sHash;

// A simple cache implementation for the (un)compressed keys.
#[cfg(feature = "cache")]
pub mod cache;
