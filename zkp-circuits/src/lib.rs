#[cfg(feature = "zkp-prover")]
pub use setup::DEFAULT_KEYS_PATH;
#[cfg(feature = "zkp-prover")]
pub use setup::DEVELOPMENT_SEED;

#[cfg(feature = "zkp-prover")]
pub mod circuits;
#[cfg(feature = "zkp-prover")]
pub(crate) mod gadgets;
#[cfg(feature = "zkp-prover")]
pub mod setup;
pub mod utils;
