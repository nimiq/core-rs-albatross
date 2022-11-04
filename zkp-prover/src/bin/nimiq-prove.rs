use nimiq_zkp_prover::prover_binary::prover_main;

#[tokio::main]
async fn main() {
    eprintln!("Starting proof generation");
    prover_main().await.unwrap();
}
