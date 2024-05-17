use std::{
    fs::{DirBuilder, File},
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use ark_groth16::Proof;
use ark_serialize::CanonicalSerialize;
use log::{info, metadata::LevelFilter};
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_log::TargetsExt;
use nimiq_primitives::policy::Policy;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer, blockchain_with_rng::produce_macro_blocks_with_rng,
    test_rng::test_rng,
};
use nimiq_zkp::prove::prove;
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

/// Generates a proof for a chain of election blocks. The random parameters generation uses always
/// the same seed, so it will always generate the same data (validators, signatures, etc).
/// This function will simply output a proof for the final epoch and store it in file.
/// Run this example with `cargo run --all-features --release --example prove`.
fn main() {
    initialize();
    // Ask user for the number of epochs.
    println!("Enter the number of epochs to prove:");

    let mut data = String::new();

    io::stdin()
        .read_line(&mut data)
        .expect("Couldn't read user input.");

    let number_epochs: u32 = data.trim().parse().expect("Couldn't read user input.");

    println!("====== Proof generation for Nano Sync initiated ======");

    let mut genesis_header_hash = [0; 32];
    let mut genesis_data = None;
    let mut proof = Proof::default();

    info!(
        "Generating blocks: {} * {}",
        number_epochs,
        Policy::blocks_per_epoch()
    );

    let block_producer = TemporaryBlockProducer::new();
    produce_macro_blocks_with_rng(
        &block_producer.producer,
        &block_producer.blockchain,
        number_epochs as usize * Policy::batches_per_epoch() as usize,
        &mut test_rng(true),
    );

    let offset = Policy::genesis_block_number();
    let path = &PathBuf::from(DEFAULT_EXAMPLE_PATH);
    let total_start = Instant::now();

    for i in 0..number_epochs {
        // Get random parameters.
        let blockchain_rg = block_producer.blockchain.read();
        let prev_block = blockchain_rg
            .get_block_at(offset + i * Policy::blocks_per_epoch(), true, None)
            .unwrap()
            .unwrap_macro();
        let final_block = blockchain_rg
            .get_block_at(offset + (i + 1) * Policy::blocks_per_epoch(), true, None)
            .unwrap()
            .unwrap_macro();

        // Create genesis data.
        if i == 0 {
            genesis_header_hash = prev_block.hash_blake2s().0;
        } else {
            genesis_data = Some((proof, genesis_header_hash))
        }

        println!("Proving epoch {}", i + 1);
        let proving_start = Instant::now();

        // Generate proof.
        proof = prove(
            prev_block,
            final_block,
            genesis_data.clone(),
            true,
            true,
            path,
        )
        .unwrap();

        let proofs_path = format!("{}/{}", DEFAULT_EXAMPLE_PATH, "proofs");
        if !Path::new(&proofs_path).is_dir() {
            DirBuilder::new().create(proofs_path.clone()).unwrap();
        }

        // Save proof to file.
        let mut file = File::create(format!("{}/proof_epoch_{}.bin", proofs_path, i + 1)).unwrap();
        proof.serialize_uncompressed(&mut file).unwrap();
        file.sync_all().unwrap();

        println!(
            "Proof generated! Elapsed time: {:?}",
            proving_start.elapsed()
        );
    }

    println!("====== Proof generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", total_start.elapsed());
}
