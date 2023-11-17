use std::{path::PathBuf, time::Instant};

use nimiq_genesis::NetworkInfo;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy::{Policy, TEST_POLICY};
use nimiq_zkp_circuits::setup::setup;
use rand::thread_rng;

/// Generates the parameters (proving and verifying keys) for the entire zkp circuit.
/// This function will store the parameters in file.
/// Run this example with `cargo run --all-features --release --example setup`.
fn main() {
    // Run tests with different policy values:
    let mut policy_config = TEST_POLICY;
    // The genesis block number must be set accordingly
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    policy_config.genesis_block_number = genesis_block.block_number();

    let _ = Policy::get_or_init(policy_config);

    println!("====== Parameter generation for ZKP Circuit initiated ======");
    let start = Instant::now();

    // use the current directory
    setup(thread_rng(), &PathBuf::new(), NetworkId::DevAlbatross, true).unwrap();

    println!("====== Parameter generation for ZKP Circuit finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}
