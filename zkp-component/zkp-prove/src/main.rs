use std::{io, path::PathBuf, sync::Arc, time::Instant};

use clap::Parser;
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
    zkp_test_data::get_base_seed,
};
use nimiq_utils::time::OffsetTime;
use nimiq_zkp::ZKP_VERIFYING_DATA;
use nimiq_zkp_circuits::setup::load_verifying_data;
use nimiq_zkp_component::{
    proof_gen_utils::generate_new_proof, proof_utils::validate_proof, types::ZKPState,
};
use nimiq_zkp_primitives::NanoZKPError;
use parking_lot::RwLock;
use tracing_subscriber::{filter::Targets, prelude::*};

/// Run the zk proof generation.
#[derive(Debug, Parser)]
struct TestProving {
    /// Network ID to utilize.
    /// Only Albatross network ids are supported.
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

#[tokio::main]
async fn main() -> Result<(), NanoZKPError> {
    let args = TestProving::parse();
    let network_id = args.network_id;

    initialize(network_id);

    // Generates the verifying keys if they don't exist yet.
    log::info!("====== Test ZK proof generation initiated ======");
    let start = Instant::now();
    produce_two_consecutive_valid_zk_proofs(network_id).await;

    log::info!("====== Test ZK proof generation finished ======");
    log::info!("Total time elapsed: {:?} seconds", start.elapsed());

    Ok(())
}

fn blockchain(network_id: NetworkId) -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(env, BlockchainConfig::default(), network_id, time).unwrap(),
    ))
}

async fn produce_two_consecutive_valid_zk_proofs(network_id: NetworkId) {
    let keys_path = PathBuf::from(format!("../{}", network_id.default_zkp_path().unwrap()));

    ZKP_VERIFYING_DATA.init_with_data(load_verifying_data(&keys_path).unwrap());
    let blockchain = blockchain(network_id);

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
    let proving_start = Instant::now();
    // Waits for the proof generation and verifies the proof.
    let zkp_state = generate_new_proof(
        zkp_state.latest_block,
        zkp_state.latest_proof,
        block,
        genesis_header_hash,
        &keys_path,
    )
    .unwrap();
    println!(
        "Proof generated! Elapsed time: {:?}",
        proving_start.elapsed()
    );

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
    let proving_start = Instant::now();
    let zkp_state = generate_new_proof(
        zkp_state.latest_block,
        zkp_state.latest_proof,
        block,
        genesis_header_hash,
        &keys_path,
    )
    .unwrap();
    println!(
        "Proof generated! Elapsed time: {:?}",
        proving_start.elapsed()
    );

    let proof = zkp_state.into();
    log::info!(
        "Proof validation: {:?}",
        validate_proof(&BlockchainProxy::from(&blockchain), &proof, None)
    );
    log::info!("Proof 2: {:?}", hex::encode(proof.serialize_to_vec()));
}
