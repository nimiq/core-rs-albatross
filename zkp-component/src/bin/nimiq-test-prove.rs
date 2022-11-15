use nimiq_primitives::policy::{Policy, TEST_POLICY};
use nimiq_zkp_component::prover_binary::prover_main;

/// This binary is only used in tests.
#[tokio::main]
async fn main() {
    let _ = Policy::get_or_init(TEST_POLICY);
    eprintln!("Starting proof generation");
    prover_main().await.unwrap();
}
