use std::{path::PathBuf, time::Instant};

use nimiq_primitives::networks::NetworkId;
use nimiq_zkp_circuits::setup::setup;
use rand::thread_rng;

/// Generates the parameters (proving and verifying keys) for the entire zkp circuit.
/// This function will store the parameters in file.
/// Run this example with `cargo run --all-features --release --example setup`.
fn main() {
    println!("====== Parameter generation for ZKP Circuit initiated ======");
    let start = Instant::now();

    // use the current directory
    setup(thread_rng(), &PathBuf::new(), NetworkId::DevAlbatross, true).unwrap();

    println!("====== Parameter generation for ZKP Circuit finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}
