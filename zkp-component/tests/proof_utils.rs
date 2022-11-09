use std::path::Path;
use std::sync::Arc;

use ark_groth16::Proof;
use beserial::Deserialize;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_primitives::policy::Policy;
use nimiq_test_utils::blockchain_with_rng::produce_macro_blocks_with_rng;
use nimiq_test_utils::zkp_test_data::{KEYS_PATH, ZKPROOF_SERIALIZED_IN_HEX};
use nimiq_zkp_component::proof_utils::ProofStore;
use nimiq_zkp_component::types::ZKProof;
use parking_lot::RwLock;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_nano_zkp::NanoZKP;
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{signing_key, voting_key};
use nimiq_test_utils::zkp_test_data::get_base_seed;
use nimiq_utils::time::OffsetTime;

use nimiq_zkp_component::proof_utils::validate_proof;

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
async fn can_detect_valid_and_invalid_genesis_proof() {
    NanoZKP::setup(get_base_seed(), Path::new(KEYS_PATH), false).unwrap();
    let blockchain = BlockchainProxy::from(blockchain());

    let proof = ZKProof {
        block_number: 0,
        proof: None,
    };
    assert!(
        validate_proof(&blockchain, &proof, None, Path::new(KEYS_PATH)),
        "The validation of a empty proof for the genesis block should succeed"
    );

    let proof = ZKProof {
        block_number: 0,
        proof: Some(Proof::default()),
    };
    assert!(
        !validate_proof(&blockchain, &proof, None, Path::new(KEYS_PATH)),
        "The validation of a Some() proof for a genesis block should fail"
    );
}

#[test(tokio::test)]
async fn can_detect_invalid_proof_none_genesis_blocks() {
    NanoZKP::setup(get_base_seed(), Path::new(KEYS_PATH), false).unwrap();
    let blockchain = blockchain();

    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    // Gets the new election block and makes a fake proof for it.
    let block = blockchain.read().state.election_head.clone();

    let zkp_proof = ZKProof {
        block_number: block.block_number(),
        proof: Some(Proof::default()),
    };

    let blockchain = BlockchainProxy::from(blockchain);

    assert!(
        !validate_proof(&blockchain, &zkp_proof, None, Path::new(KEYS_PATH)),
        "The validation of a fake proof should fail"
    );

    let zkp_proof = ZKProof {
        block_number: block.block_number(),
        proof: None,
    };

    assert!(
        !validate_proof(&blockchain, &zkp_proof, None, Path::new(KEYS_PATH)),
        "The validation of a empty proof for a non genesis block should fail"
    );

    let zkp_proof = ZKProof {
        block_number: block.block_number() + Policy::blocks_per_epoch(),
        proof: Some(Proof::default()),
    };

    assert!(
        !validate_proof(&blockchain, &zkp_proof, None, Path::new(KEYS_PATH)),
        "The validation of a proof for a non existing block should fail"
    );
}

#[test(tokio::test)]
async fn can_detect_valid_proof_none_genesis_blocks() {
    NanoZKP::setup(get_base_seed(), Path::new(KEYS_PATH), false).unwrap();
    let blockchain = blockchain();

    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_rng(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        &mut get_base_seed(),
    );

    // Gets the election block and sets the precomputed zk proof from it.
    let zkp_proof =
        &ZKProof::deserialize_from_vec(&hex::decode(ZKPROOF_SERIALIZED_IN_HEX).unwrap()).unwrap();
    assert!(
        validate_proof(
            &BlockchainProxy::from(blockchain),
            &zkp_proof,
            None,
            Path::new(KEYS_PATH)
        ),
        "The validation of a valid proof failed"
    );
}

#[test(tokio::test)]
async fn can_store_and_load_zkp_state_from_db() {
    let env = VolatileEnvironment::new(1).unwrap();

    let proof_store = ProofStore::new(env);
    let new_proof = ZKProof {
        block_number: Policy::blocks_per_epoch(),
        proof: Some(Proof::default()),
    };

    proof_store.set_zkp(&new_proof);
    assert_eq!(
        proof_store.get_zkp().unwrap(),
        new_proof,
        "Load from db was not successful"
    );

    let new_proof = ZKProof {
        block_number: Policy::blocks_per_epoch(),
        proof: Some(Proof::default()),
    };

    proof_store.set_zkp(&new_proof);
    assert_eq!(
        proof_store.get_zkp().unwrap(),
        new_proof,
        "Load from db was not successful"
    );
}
