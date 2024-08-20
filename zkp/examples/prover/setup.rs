use std::{io, path::PathBuf, time::Instant};

use log::metadata::LevelFilter;
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_log::TargetsExt;
use nimiq_primitives::policy::Policy;
use nimiq_zkp_circuits::setup::setup;
use rand::thread_rng;
use tracing_subscriber::{filter::Targets, prelude::*};

const DEFAULT_EXAMPLE_PATH: &str = ".zkp_example";

fn initialize() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .with(
            Targets::new()
                .with_default(LevelFilter::INFO)
                .with_nimiq_targets(LevelFilter::DEBUG)
                .with_target("r1cs", LevelFilter::WARN)
                .with_env(),
        )
        .init();
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();

    // Run tests with different policy values:
    let mut policy_config = Policy::default();
    // The genesis block number must be set accordingly
    policy_config.genesis_block_number = genesis_block.block_number();

    let _ = Policy::get_or_init(policy_config);
}

/// Generates the parameters (proving and verifying keys) for the entire zkp circuit.
/// This function will store the parameters in file.
/// Run this example with `cargo run --all-features --release --example setup`.
fn main() {
    initialize();

    println!("====== Parameter generation for ZKP Circuit initiated ======");
    let start = Instant::now();

    // use the current directory
    setup(
        thread_rng(),
        &PathBuf::from(DEFAULT_EXAMPLE_PATH),
        NetworkId::TestAlbatross,
        true,
    )
    .unwrap();

    println!("====== Parameter generation for ZKP Circuit finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}
