use std::{io, path::PathBuf, time::Instant};

use clap::Parser;
use log::level_filters::LevelFilter;
use nimiq_genesis::NetworkInfo;
use nimiq_log::TargetsExt;
use nimiq_primitives::{
    networks::NetworkId,
    policy::{Policy, TEST_POLICY},
};
use nimiq_zkp_circuits::{
    setup::{setup, DEVELOPMENT_SEED},
    test_setup::{setup_merger_wrapper_simulation, UNIT_TOXIC_WASTE_SEED},
};
use nimiq_zkp_primitives::NanoZKPError;
use rand::{thread_rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Parser)]

/// Create the zkp keys for Devnet or Unit test.
struct Setup {
    /// Network ID to generate ZKP keys for.
    /// Only supports Albatross network ids.
    #[clap(short = 'n', long, value_enum)]
    network_id: NetworkId,
}

fn initialize(network_id: NetworkId) {
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
    let network_info = NetworkInfo::from_network_id(network_id);
    let genesis_block = network_info.genesis_block();

    // Run tests with different policy values:
    let mut policy_config = match network_id {
        NetworkId::UnitAlbatross => TEST_POLICY,
        NetworkId::TestAlbatross | NetworkId::DevAlbatross | NetworkId::MainAlbatross => {
            Policy::default()
        }
        _ => panic!("Invalid network id"),
    };
    // The genesis block number must be set accordingly
    policy_config.genesis_block_number = genesis_block.block_number();

    let _ = Policy::get_or_init(policy_config);
}

fn main() -> Result<(), NanoZKPError> {
    let args = Setup::parse();
    let network_id = args.network_id;

    initialize(network_id);
    let keys_path = PathBuf::from(&network_id.default_zkp_path().unwrap());

    // Generates the verifying keys if they don't exist yet.
    println!("====== Devnet Parameter generation for ZKP initiated ======");
    println!("Starting keys setup at: {keys_path:?}");
    let start = Instant::now();

    match network_id {
        NetworkId::UnitAlbatross | NetworkId::DevAlbatross => setup(
            ChaCha20Rng::from_seed(DEVELOPMENT_SEED),
            &keys_path,
            network_id,
            true,
        )
        .unwrap(),
        NetworkId::TestAlbatross | NetworkId::MainAlbatross => {
            setup(thread_rng(), &keys_path, network_id, true).unwrap()
        }
        _ => panic!("Invalid network id."),
    };
    if matches!(network_id, NetworkId::UnitAlbatross) {
        setup_merger_wrapper_simulation(
            &mut ChaCha20Rng::from_seed(UNIT_TOXIC_WASTE_SEED),
            &keys_path,
        )
        .unwrap();
    }

    println!("====== Devnet Parameter generation for ZKP finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());

    Ok(())
}
