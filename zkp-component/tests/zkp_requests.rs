use std::{path::Path, sync::Arc};

use futures::StreamExt;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_database::volatile::VolatileDatabase;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{signing_key, voting_key},
    blockchain_with_rng::produce_macro_blocks_with_rng,
    zkp_test_data::{get_base_seed, simulate_merger_wrapper, ZKP_TEST_KEYS_PATH},
};
use nimiq_utils::time::OffsetTime;
use example::ZKP_VERIFYING_DATA;
use nimiq_zkp_component::{
    proof_store::{DBProofStore, ProofStore},
    proof_utils::validate_proof,
    zkp_requests::ZKPRequests,
    ZKPComponent,
};
use parking_lot::RwLock;

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileDatabase::new(20).unwrap();
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
async fn peers_do_not_reply_with_outdated_proof() {
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let network2 = Arc::new(hub.new_network());
    let network3 = Arc::new(hub.new_network());
    network.dial_address(network3.address()).await.unwrap();
    network.dial_address(network2.address()).await.unwrap();

    let _zkp_prover2 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain),
        Arc::clone(&network2),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        None,
    )
    .await;

    let _zkp_prover3 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain),
        Arc::clone(&network3),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        None,
    )
    .await;

    let mut zkp_requests = ZKPRequests::new(Arc::clone(&network));

    // Trigger zkp requests
    zkp_requests.request_zkps(network.get_peers(), Policy::genesis_block_number(), false);

    for _ in 0..2 {
        assert!(
            zkp_requests.next().await.is_none(),
            "Peer sent a proof when it should have abstained because of having an outdated proof"
        );
    }
}

#[test(tokio::test)]
async fn peers_reply_with_valid_proof() {
    let blockchain2 = blockchain();
    let blockchain3 = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let network2 = Arc::new(hub.new_network());
    let network3 = Arc::new(hub.new_network());
    network.dial_address(network3.address()).await.unwrap();
    network.dial_address(network2.address()).await.unwrap();

    let env2 = VolatileDatabase::new(20).unwrap();
    let env3 = VolatileDatabase::new(20).unwrap();
    let store2 = DBProofStore::new(env2.clone());
    let store3 = DBProofStore::new(env3.clone());
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain2,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain3,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    // Set a valid proof into the 2 components.
    let new_proof = simulate_merger_wrapper(
        Path::new(ZKP_TEST_KEYS_PATH),
        &blockchain2,
        &ZKP_VERIFYING_DATA,
        &mut get_base_seed(),
    );
    log::info!("setting proof");
    store2.set_zkp(&new_proof);
    store3.set_zkp(&new_proof);

    log::info!("launching zkps");
    let proof_store_2: Option<Box<dyn ProofStore>> = Some(Box::new(store2));
    let _zkp_prover2 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain2),
        Arc::clone(&network2),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        proof_store_2,
    )
    .await;
    let proof_store_3: Option<Box<dyn ProofStore>> = Some(Box::new(store3));
    let _zkp_prover3 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain3),
        Arc::clone(&network3),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        proof_store_3,
    )
    .await;

    let mut zkp_requests = ZKPRequests::new(Arc::clone(&network));

    // Trigger zkp requests from the first component.
    zkp_requests.request_zkps(network.get_peers(), 0, false);

    for _ in 0..2 {
        let proof_data = zkp_requests.next().await.unwrap();
        assert!(
            proof_data.election_block.is_none(),
            "Peers should not send an election block"
        );
        assert!(
            validate_proof(
                &BlockchainProxy::from(&blockchain2),
                &proof_data.proof,
                None,
            ),
            "Peer should sent a new proof valid proof"
        );
    }
}

#[test(tokio::test)]
async fn peers_reply_with_valid_proof_and_election_block() {
    let blockchain2 = blockchain();
    let blockchain3 = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let network2 = Arc::new(hub.new_network());
    let network3 = Arc::new(hub.new_network());
    network.dial_address(network3.address()).await.unwrap();
    network.dial_address(network2.address()).await.unwrap();

    let env2 = VolatileDatabase::new(20).unwrap();
    let env3 = VolatileDatabase::new(20).unwrap();
    let store2 = DBProofStore::new(env2.clone());
    let store3 = DBProofStore::new(env3.clone());
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain2,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain3,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    // Seta valid proof into the 2 components.
    let new_proof = simulate_merger_wrapper(
        Path::new(ZKP_TEST_KEYS_PATH),
        &blockchain2,
        &ZKP_VERIFYING_DATA,
        &mut get_base_seed(),
    );
    log::info!("setting proof");
    store2.set_zkp(&new_proof);
    store3.set_zkp(&new_proof);

    log::info!("launching zkps");
    let proof_store_2: Option<Box<dyn ProofStore>> = Some(Box::new(store2));
    let _zkp_prover2 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain2),
        Arc::clone(&network2),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        proof_store_2,
    )
    .await;
    let proof_store_3: Option<Box<dyn ProofStore>> = Some(Box::new(store3));
    let _zkp_prover3 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain3),
        Arc::clone(&network3),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        proof_store_3,
    )
    .await;

    let mut zkp_requests = ZKPRequests::new(Arc::clone(&network));

    // Trigger zkp requests from the first component.
    zkp_requests.request_zkps(network.get_peers(), 0, true);

    for _ in 0..2 {
        let proof_data = zkp_requests.next().await.unwrap();
        assert!(
            proof_data.election_block.is_some(),
            "Peers should send an election block"
        );
        assert!(
            validate_proof(
                &BlockchainProxy::from(&blockchain2),
                &proof_data.proof,
                proof_data.election_block,
            ),
            "Peer should sent a new proof valid proof"
        );
    }
}
