use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{signing_key, voting_key},
    blockchain_with_rng::produce_macro_blocks_with_rng,
    zkp_test_data::{get_base_seed, zkp_test_exe, ZKPROOF_SERIALIZED_IN_HEX, ZKP_TEST_KEYS_PATH},
};
use nimiq_utils::time::OffsetTime;
use nimiq_zkp_circuits::setup::setup;

use nimiq_zkp_component::proof_store::{DBProofStore, ProofStore};
use nimiq_zkp_component::proof_utils::validate_proof;
use nimiq_zkp_component::types::ZKProof;
use nimiq_zkp_component::types::ZKProofTopic;
use nimiq_zkp_component::zkp_component::ZKPComponent;
use nimiq_zkp_component::zkp_component::ZKProofsStream;

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
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

#[test(tokio::test)]
async fn builds_valid_genesis_proof() {
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let zkp_prover = ZKPComponent::new(
        BlockchainProxy::from(&blockchain),
        Arc::clone(&network),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        None,
    )
    .await
    .proxy();

    assert!(
        validate_proof(
            &BlockchainProxy::from(blockchain),
            &zkp_prover.get_zkp_state().into(),
            None,
        ),
        "The validation of a empty proof for the genesis block should succeed"
    );
}

#[test(tokio::test)]
#[ignore]
async fn loads_valid_zkp_state_from_db() {
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let proof_store = DBProofStore::new(VolatileEnvironment::new(1).unwrap());
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    let new_proof =
        &ZKProof::deserialize_from_vec(&hex::decode(ZKPROOF_SERIALIZED_IN_HEX).unwrap()).unwrap();

    proof_store.set_zkp(&new_proof);

    let proof_store: Option<Box<dyn ProofStore>> = Some(Box::new(proof_store));
    let zkp_prover = ZKPComponent::new(
        BlockchainProxy::from(&blockchain),
        Arc::clone(&network),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        proof_store,
    )
    .await;

    let zkp_proxy = zkp_prover.proxy();
    assert_eq!(
        zkp_proxy.get_zkp_state().latest_block_number,
        Policy::blocks_per_epoch(),
        "The load of the zkp state should have worked"
    );
}

#[test(tokio::test)]
async fn does_not_load_invalid_zkp_state_from_db() {
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let env = VolatileEnvironment::new(1).unwrap();

    let proof_store = DBProofStore::new(env);
    let new_proof = ZKProof {
        block_number: Policy::blocks_per_epoch(),
        proof: None,
    };

    proof_store.set_zkp(&new_proof);

    let proof_store: Option<Box<dyn ProofStore>> = Some(Box::new(proof_store));
    let zkp_prover = ZKPComponent::new(
        BlockchainProxy::from(&blockchain),
        Arc::clone(&network),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        proof_store,
    )
    .await;

    let zkp_proxy = zkp_prover.proxy();
    assert_eq!(
        zkp_proxy.get_zkp_state().latest_block_number,
        0,
        "The load of the zkp state should have failed"
    );
}

#[test(tokio::test)]
#[ignore]
async fn can_produce_two_consecutive_valid_zk_proofs() {
    setup(get_base_seed(), Path::new(ZKP_TEST_KEYS_PATH), true).unwrap();
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let network2 = Arc::new(hub.new_network());
    network2.dial_address(network.address()).await.unwrap();
    network.dial_address(network2.address()).await.unwrap();

    let zkp_prover = ZKPComponent::with_prover(
        BlockchainProxy::from(&blockchain),
        Arc::clone(&network),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        true,
        Some(zkp_test_exe()),
        PathBuf::from(ZKP_TEST_KEYS_PATH),
        None,
    )
    .await;

    tokio::spawn(zkp_prover);

    assert_eq!(blockchain.read().block_number(), 0);

    let mut zk_proofs_stream: ZKProofsStream<MockNetwork> =
        network2.subscribe::<ZKProofTopic>().await.unwrap().boxed();
    // Produce the 1st election block after genesis.
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    log::info!("Going to wait for the 1st proof");

    // Waits for the proof generation and verifies the proof.
    let (proof, _) = zk_proofs_stream.as_mut().next().await.unwrap();
    assert!(
        validate_proof(&BlockchainProxy::from(&blockchain), &proof, None,),
        "Generated ZK proof for the first block should be valid"
    );

    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    log::info!("Going to wait for the 2nd proof");

    let (proof, _) = zk_proofs_stream.as_mut().next().await.unwrap();
    assert!(
        validate_proof(&BlockchainProxy::from(&blockchain), &proof, None,),
        "Generated ZK proof for the second block should be valid"
    );
}
