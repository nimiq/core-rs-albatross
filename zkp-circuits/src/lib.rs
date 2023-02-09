#[cfg(feature = "zkp-prover")]
mod blake2s;
#[cfg(feature = "zkp-prover")]
pub mod circuits;
pub mod endianness;
#[cfg(feature = "zkp-prover")]
pub(crate) mod gadgets;
#[cfg(feature = "zkp-prover")]
pub mod setup;
pub mod utils;

pub const DEFAULT_KEYS_PATH: &str = ".zkp";
