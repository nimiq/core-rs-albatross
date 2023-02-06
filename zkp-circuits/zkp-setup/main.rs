use std::{path::Path, time::Instant};

use clap::{Arg, Command};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

use nimiq_zkp_circuits::{
    setup::{setup, DEFAULT_KEYS_PATH},
    DEVELOPMENT_SEED,
};
use nimiq_zkp_primitives::NanoZKPError;

fn main() -> Result<(), NanoZKPError> {
    let matches = Command::new("nimiq-zkp-setup")
        .about("Create the zkp keys for Devnet.")
        .arg(
            Arg::new("path")
                .value_name("PATH")
                .default_value(DEFAULT_KEYS_PATH),
        )
        .get_matches();

    let keys_path = if let Some(p) = matches.get_one::<String>("path") {
        Path::new(p)
    } else {
        Path::new(DEFAULT_KEYS_PATH)
    };

    // Generates the verifying keys if they don't exist yet.
    println!("====== Devnet Parameter generation for ZKP initiated ======");
    println!("Starting keys setup at: {keys_path:?}");
    let start = Instant::now();

    setup(ChaCha20Rng::from_seed(DEVELOPMENT_SEED), keys_path, true).unwrap();

    println!("====== Devnet Parameter generation for ZKP finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());

    Ok(())
}
