use std::{io, path::Path, sync::Arc, time::Instant};

use log::metadata::LevelFilter;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis::NetworkInfo;
use nimiq_log::TargetsExt;
use nimiq_primitives::{
    networks::NetworkId,
    policy::{Policy, TEST_POLICY},
};
use nimiq_serde::Serialize;
use nimiq_test_utils::{
    blockchain::{signing_key, voting_key},
    blockchain_with_rng::produce_macro_blocks_with_rng,
    zkp_test_data::{get_base_seed, DEFAULT_TEST_KEYS_PATH},
};
use nimiq_utils::time::OffsetTime;
use nimiq_zkp::ZKP_VERIFYING_DATA;
use nimiq_zkp_circuits::setup::{load_verifying_data, setup};
use nimiq_zkp_component::{
    proof_gen_utils::generate_new_proof, proof_utils::validate_proof, types::ZKPState,
};
use nimiq_zkp_primitives::NanoZKPError;
use parking_lot::RwLock;
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
    // Shorter epochs and shorter batches
    let _ = Policy::get_or_init(TEST_POLICY);
}

#[tokio::main]
async fn main() -> Result<(), NanoZKPError> {
    initialize();
    // Generates the verifying keys if they don't exist yet.
    log::info!("====== Test ZK proof generation initiated ======");
    let start = Instant::now();
    produce_two_consecutive_valid_zk_proofs().await;

    log::info!("====== Test ZK proof generation finished ======");
    log::info!("Total time elapsed: {:?} seconds", start.elapsed());

    Ok(())
}

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ))
}

async fn produce_two_consecutive_valid_zk_proofs() {
    setup(
        get_base_seed(),
        Path::new(DEFAULT_TEST_KEYS_PATH),
        NetworkId::UnitAlbatross,
        true,
    )
    .unwrap();
    ZKP_VERIFYING_DATA
        .init_with_data(load_verifying_data(Path::new(DEFAULT_TEST_KEYS_PATH)).unwrap());

    let blockchain = blockchain();

    // Produce the 1st election block after genesis.
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    let block = blockchain.read().state.election_head.clone();
    let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
    let genesis_block = network_info.genesis_block().unwrap_macro();
    let zkp_state = ZKPState::with_genesis(&genesis_block).expect("Invalid genesis block");

    let genesis_header_hash = genesis_block.hash_blake2s().0;

    log::info!("Going to wait for the 1st proof");
    // Waits for the proof generation and verifies the proof.
    let zkp_state = generate_new_proof(
        zkp_state.latest_block,
        zkp_state.latest_proof,
        block,
        genesis_header_hash,
        Path::new(DEFAULT_TEST_KEYS_PATH),
    )
    .unwrap();
    let proof = zkp_state.clone().into();

    log::info!(
        "Proof validation: {:?}",
        validate_proof(&BlockchainProxy::from(&blockchain), &proof, None)
    );
    log::info!("Proof 1: {:?}", hex::encode(proof.serialize_to_vec()));

    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    let block = blockchain.read().state.election_head.clone();

    log::info!("Going to wait for the 2nd proof");

    let zkp_state = generate_new_proof(
        zkp_state.latest_block,
        zkp_state.latest_proof,
        block,
        genesis_header_hash,
        Path::new(DEFAULT_TEST_KEYS_PATH),
    )
    .unwrap();
    let proof = zkp_state.into();

    log::info!(
        "Proof validation: {:?}",
        validate_proof(&BlockchainProxy::from(&blockchain), &proof, None)
    );
    log::info!("Proof 2: {:?}", hex::encode(proof.serialize_to_vec()));
}
