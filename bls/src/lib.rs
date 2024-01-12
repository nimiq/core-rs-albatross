use nimiq_hash::Blake2sHash;
pub use types::*;

// Implements the LazyPublicKey type. Which is a faster, cached version of PublicKey.
#[cfg(feature = "lazy")]
pub mod lazy;

// Implements all of the types needed to do BLS signatures.
mod types;

// Specifies the hash algorithm used for signatures
pub type SigHash = Blake2sHash;

// A simple cache implementation for the (un)compressed keys.
#[cfg(feature = "cache")]
pub mod cache;

// Implements the tagged-signing traits
mod tagged_signing;
