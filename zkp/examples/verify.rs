use std::{fs::File, io, path::PathBuf, time::Instant};

use ark_groth16::Proof;
use ark_serialize::CanonicalDeserialize;
use log::metadata::LevelFilter;
use nimiq_genesis::NetworkInfo;
use nimiq_log::TargetsExt;
use nimiq_primitives::{
    networks::NetworkId,
    policy::{Policy, TEST_POLICY},
};
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer, blockchain_with_rng::produce_macro_blocks_with_rng,
    test_rng::test_rng,
};
use nimiq_zkp::{verify::verify, ZKP_VERIFYING_DATA};
use nimiq_zkp_circuits::setup::load_verifying_data;
use tracing_subscriber::{filter::Targets, prelude::*};

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

    // Run tests with different policy values:
    let mut policy_config = Policy::default();
    // The genesis block number must be set accordingly
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    policy_config.genesis_block_number = genesis_block.block_number();

    let _ = Policy::get_or_init(policy_config);
}

/// Verifies a proof for a chain of election blocks. The random parameters generation uses always
/// the same seed, so it will always generate the same data (validators, signatures, etc).
/// This function will simply print the verification result.
/// Run this example with `cargo run --release --example verify`.
fn main() {
    initialize();
    // use the current directory
    ZKP_VERIFYING_DATA.init_with_data(
        load_verifying_data(&PathBuf::new()).expect("No keys in current directory"),
    );

    // Ask user for the number of epochs.
    println!("Enter the number of epochs to verify:");

    let mut data = String::new();

    io::stdin()
        .read_line(&mut data)
        .expect("Couldn't read user input.");

    let number_epochs: u32 = data.trim().parse().expect("Couldn't read user input.");

    println!("====== Generating random inputs ======");
    let block_producer = TemporaryBlockProducer::new();

    let offset = Policy::genesis_block_number();

    // Get initial random parameters.
    produce_macro_blocks_with_rng(
        &block_producer.producer,
        &block_producer.blockchain,
        number_epochs as usize * Policy::batches_per_epoch() as usize,
        &mut test_rng(true),
    );

    let blockchain_rg = block_producer.blockchain.read();
    let genesis_header_hash = blockchain_rg
        .get_block_at(offset, true, None)
        .unwrap()
        .unwrap_macro()
        .hash_blake2s();
    // Get final random parameters.
    let final_header_hash = blockchain_rg
        .get_block_at(
            offset + number_epochs * Policy::blocks_per_epoch(),
            true,
            None,
        )
        .unwrap()
        .unwrap_macro()
        .hash_blake2s();

    // Load the proof from file.
    let mut file = File::open(format!("proofs/proof_epoch_{number_epochs}.bin")).unwrap();

    let proof = Proof::deserialize_uncompressed_unchecked(&mut file).unwrap();

    println!("====== Proof verification for Nano Sync initiated ======");

    let start = Instant::now();

    // Verify proof.
    let result = verify(
        genesis_header_hash,
        final_header_hash,
        proof,
        &ZKP_VERIFYING_DATA,
    )
    .unwrap();

    println!("Proof verification finished. It returned {result}.");

    println!("====== Proof verification for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}
