use std::sync::Arc;

use futures::StreamExt;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::MockHub;
use nimiq_network_mock::MockNetwork;
use nimiq_primitives::policy;
use nimiq_zkp_prover::proof_component::ProofStore;
use nimiq_zkp_prover::types::ZKPState;

use nimiq_zkp_prover::types::ZKProofTopic;
use parking_lot::RwLock;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_nano_zkp::NanoZKP;
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_utils::time::OffsetTime;

use nimiq_zkp_prover::proof_component as ZKProofComponent;
use nimiq_zkp_prover::zkp_component::ZKPComponent;
use nimiq_zkp_prover::zkp_component::ZKProofsStream;

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ))
}

#[test(tokio::test)]
async fn builds_valid_genesis_proof() {
    NanoZKP::setup().unwrap();
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let env = VolatileEnvironment::new(10).unwrap();

    let zkp_prover = ZKPComponent::new(Arc::clone(&blockchain), Arc::clone(&network), false, env)
        .await
        .proxy();

    assert!(
        ZKProofComponent::validate_proof(&blockchain, zkp_prover.get_zkp_state().into()),
        "The validation of a empty proof for the genesis block should successed"
    );
}

#[test(tokio::test)]
async fn does_not_load_invalid_zkp_state_from_db() {
    NanoZKP::setup().unwrap();
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let env = VolatileEnvironment::new(1).unwrap();

    let proof_store = ProofStore::new(env);
    let new_zkp_state = ZKPState {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: policy::BLOCKS_PER_EPOCH,
        latest_proof: None,
    };

    proof_store.set_zkp(&new_zkp_state);
    assert_eq!(
        proof_store.get_zkp().unwrap().latest_block_number,
        new_zkp_state.latest_block_number,
        "Load from db was not succesfull"
    );

    let zkp_prover = ZKPComponent::new(
        Arc::clone(&blockchain),
        Arc::clone(&network),
        false,
        proof_store.env,
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
async fn can_produce_valid_zk_proofs() {
    NanoZKP::setup().unwrap();
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let network2 = Arc::new(hub.new_network());
    network2.dial_address(network.address()).await.unwrap();
    network.dial_address(network2.address()).await.unwrap();

    let env = VolatileEnvironment::new(10).unwrap();

    let zkp_prover =
        ZKPComponent::new(Arc::clone(&blockchain), Arc::clone(&network), true, env).await;
    tokio::spawn(zkp_prover);

    assert_eq!(blockchain.read().block_number(), 0);

    // Produce the 1st election block after genesis
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &blockchain, policy::BATCHES_PER_EPOCH as usize);
    let mut zk_proofs_stream: ZKProofsStream<MockNetwork> =
        network2.subscribe::<ZKProofTopic>().await.unwrap().boxed();

    // Waits for the proof generation and verifies the proof
    match zk_proofs_stream.as_mut().next().await {
        Some((proof, _)) => {
            assert!(
                ZKProofComponent::validate_proof(&blockchain, proof),
                "Invalid zk proof of first election block"
            );
        }
        None => {}
    }

    produce_macro_blocks(&producer, &blockchain, policy::BATCHES_PER_EPOCH as usize);

    match zk_proofs_stream.as_mut().next().await {
        Some((proof, _)) => {
            log::error!("going to validate 2nd proof");
            let val = ZKProofComponent::validate_proof(&blockchain, proof);
            log::error!("validation = {}", val);
            assert!(val, "Invalid zk proof of second election block");
            log::error!("validated successfully");
        }
        None => {}
    }
    log::error!("it's over");
}
