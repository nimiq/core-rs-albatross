use nimiq_zkp_component::prover_binary::prover_main;

#[tokio::main]
async fn main() {
    eprintln!("Starting proof generation");
    prover_main().await.unwrap();
}
