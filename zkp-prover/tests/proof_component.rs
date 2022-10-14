use std::sync::Arc;

use ark_groth16::Proof;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;
use nimiq_zkp_prover::proof_component::ProofStore;
use nimiq_zkp_prover::types::ZKPState;
use nimiq_zkp_prover::types::ZKProof;
use parking_lot::RwLock;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::Blockchain;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_nano_zkp::NanoZKP;
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_utils::time::OffsetTime;

use nimiq_zkp_prover::proof_component as ZKProofComponent;

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ))
}

#[test(tokio::test)]
async fn can_detect_valid_and_invalid_genesis_proof() {
    let blockchain = blockchain();

    let proof = ZKProof {
        block_number: 0,
        proof: None,
    };
    assert!(
        ZKProofComponent::validate_proof(&blockchain, proof),
        "The validation of a empty proof for the genesis block should successed"
    );

    let proof = ZKProof {
        block_number: 0,
        proof: Some(Proof::default()),
    };
    assert!(
        !ZKProofComponent::validate_proof(&blockchain, proof),
        "The validation of a Some() proof for a genesis block should fail"
    );
}

#[test(tokio::test)]
async fn can_detect_invalid_proof_none_genesis_blocks() {
    NanoZKP::setup().unwrap();
    let blockchain = blockchain();

    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &blockchain, policy::BATCHES_PER_EPOCH as usize);

    // Gets the new election block and makes a fake proof for it
    let block = blockchain.read().state.election_head.clone();

    let zkp_proof = ZKProof {
        block_number: block.block_number(),
        proof: Some(Proof::default()),
    };

    assert!(
        !ZKProofComponent::validate_proof(&blockchain, zkp_proof),
        "The validation of a fake proof should failed"
    );

    let zkp_proof = ZKProof {
        block_number: block.block_number(),
        proof: None,
    };

    assert!(
        !ZKProofComponent::validate_proof(&blockchain, zkp_proof),
        "The validation of a empty proof for a non genesis block should fail"
    );

    let zkp_proof = ZKProof {
        block_number: block.block_number() + policy::BLOCKS_PER_EPOCH,
        proof: Some(Proof::default()),
    };

    assert!(
        !ZKProofComponent::validate_proof(&blockchain, zkp_proof),
        "The validation of a proof for a non existing block should fail"
    );
}

#[test(tokio::test)]
async fn can_store_and_load_zkp_state() {
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
        proof_store.get_zkp().unwrap(),
        new_zkp_state,
        "Load from db was not succesfull"
    );

    let new_zkp_state = ZKPState {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: policy::BLOCKS_PER_EPOCH,
        latest_proof: Some(Proof::default()),
    };

    proof_store.set_zkp(&new_zkp_state);
    assert_eq!(
        proof_store.get_zkp().unwrap(),
        new_zkp_state,
        "Load from db was not succesfull"
    );
}
