pub mod client;
pub mod config;
pub mod error;
pub mod extras;

pub mod prover {
    pub use nimiq_zkp_prover::prover_binary::prover_main;
}
