use std::time::Instant;

use nimiq_nano_zkp::NanoZKP;

/// Generates the parameters (proving and verifying keys) for the entire nano sync circuit.
/// This function will store the parameters in file.
/// Run this example with `cargo run --all-features --release --example setup`.
fn main() {
    println!("====== Parameter generation for Nano Sync initiated ======");
    let start = Instant::now();

    NanoZKP::setup().unwrap();

    println!("====== Parameter generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}
