use std::{path::Path, sync::Arc};

use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_database::volatile::VolatileDatabase;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{signing_key, voting_key},
    blockchain_with_rng::produce_macro_blocks_with_rng,
    zkp_test_data::{get_base_seed, simulate_merger_wrapper, ZKP_TEST_KEYS_PATH},
};
use nimiq_utils::time::OffsetTime;
use nimiq_zkp::ZKP_VERIFYING_DATA;
use nimiq_zkp_component::{
    proof_store::{DBProofStore, ProofStore},
    proof_utils::validate_proof,
    types::ZKProof,
    zkp_component::ZKPComponent,
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
async fn loads_valid_zkp_state_from_db() {
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let proof_store = DBProofStore::new(VolatileDatabase::new(1).unwrap());
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    let new_proof = simulate_merger_wrapper(
        Path::new(ZKP_TEST_KEYS_PATH),
        &blockchain,
        &ZKP_VERIFYING_DATA,
        &mut get_base_seed(),
    );

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
        zkp_proxy.get_zkp_state().latest_block.block_number(),
        Policy::blocks_per_epoch() + Policy::genesis_block_number(),
        "The load of the zkp state should have worked"
    );
}

#[test(tokio::test)]
async fn does_not_load_invalid_zkp_state_from_db() {
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let env = VolatileDatabase::new(1).unwrap();

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
        zkp_proxy.get_zkp_state().latest_block.block_number(),
        Policy::genesis_block_number(),
        "The load of the zkp state should have failed"
    );
}
