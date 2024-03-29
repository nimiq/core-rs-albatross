pub mod client;
pub mod config;
pub mod error;
pub mod extras;

#[cfg(feature = "zkp-prover")]
pub mod prover {
    pub use nimiq_zkp_component::prover_binary::prover_main;
}
