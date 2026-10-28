pub use lazy::{LazyAggregateSignature, LazyPublicKey};
use nimiq_hash::Blake2sHash;
pub use types::*;

// Implements the LazyPublicKey and LazyAggregateSignature types, which are uncompressed on first use.
mod lazy;

// Implements all of the types needed to do BLS signatures.
mod types;

// Specifies the hash algorithm used for signatures
pub type SigHash = Blake2sHash;

// Implements the tagged-signing traits
#[cfg(feature = "serde-derive")]
mod tagged_signing;
