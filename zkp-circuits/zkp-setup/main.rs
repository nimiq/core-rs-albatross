use std::{path::Path, time::Instant};

use clap::Parser;
use nimiq_zkp_primitives::NanoZKPError;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

use nimiq_primitives::{
    networks::NetworkId,
    policy::{Policy, TEST_POLICY},
};
use nimiq_zkp_circuits::{
    setup::{setup, DEVELOPMENT_SEED},
    DEFAULT_KEYS_PATH,
};

// This is a copied constant from nimiq-test-utils.
const DEFAULT_TEST_KEYS_PATH: &str = ".zkp_tests";

#[derive(Debug, Parser)]

/// Create the zkp keys for Devnet or Unit test.
struct Setup {
    /// Network ID to generate ZKP keys for.
    /// Currently supported are: UnitAlbatross and DevAlbatross
    #[clap(short = 'n', long, value_enum)]
    network_id: Option<NetworkId>,
}

fn main() -> Result<(), NanoZKPError> {
    let args = Setup::parse();

    let keys_path = Path::new(match args.network_id.unwrap_or(NetworkId::DevAlbatross) {
        NetworkId::DevAlbatross => DEFAULT_KEYS_PATH,
        NetworkId::UnitAlbatross => {
            // Use test constants for the setup.
            let _ = Policy::get_or_init(TEST_POLICY);
            DEFAULT_TEST_KEYS_PATH
        }
        _ => panic!("Invalid network ID"),
    });

    // Generates the verifying keys if they don't exist yet.
    println!("====== Devnet Parameter generation for ZKP initiated ======");
    println!("Starting keys setup at: {keys_path:?}");
    let start = Instant::now();

    setup(ChaCha20Rng::from_seed(DEVELOPMENT_SEED), keys_path, true).unwrap();

    println!("====== Devnet Parameter generation for ZKP finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());

    Ok(())
}
