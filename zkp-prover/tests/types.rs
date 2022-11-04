use ark_groth16::Proof;
use beserial::{Deserialize, Serialize};
use nimiq_block::MacroBlock;
use nimiq_database::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;
use nimiq_zkp_prover::types::{ProofInput, ZKPState, ZKProof};

#[test]
fn it_serializes_and_deserializes_zk_proof() {
    let b = ZKProof {
        block_number: 0,
        proof: None,
    };
    let serialized = Serialize::serialize_to_vec(&b);
    let deserialized: ZKProof = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, b);

    let proof = ZKProof {
        block_number: 0,
        proof: Some(Proof::default()),
    };
    let serialized = Serialize::serialize_to_vec(&proof);
    let deserialized: ZKProof = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof);
}

#[test]
fn it_serializes_and_deserializes_to_bytes_zk_proof() {
    let proof = ZKProof {
        block_number: 0,
        proof: None,
    };
    let serialized = proof.as_database_bytes();
    let deserialized: ZKProof = FromDatabaseValue::copy_from_database(&serialized).unwrap();
    assert_eq!(deserialized, proof);

    let proof = ZKProof {
        block_number: 0,
        proof: Some(Proof::default()),
    };
    let serialized = proof.as_database_bytes();
    let deserialized: ZKProof = FromDatabaseValue::copy_from_database(&serialized).unwrap();
    assert_eq!(deserialized, proof);
}

#[test]
fn it_serializes_and_deserializes_zkp_state() {
    let state = ZKPState {
        latest_pks: vec![Default::default()],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: policy::BLOCKS_PER_EPOCH,
        latest_proof: Some(Proof::default()),
    };
    let serialized = Serialize::serialize_to_vec(&state);
    let deserialized: ZKPState = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, state);

    let state = ZKPState {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: 0,
        latest_proof: None,
    };
    let serialized = Serialize::serialize_to_vec(&state);
    let deserialized: ZKPState = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, state);
}

#[test]
fn it_serializes_and_deserializes_proof_input() {
    let proof_input = ProofInput {
        latest_pks: vec![Default::default()],
        latest_header_hash: Blake2bHash::default(),
        block: MacroBlock::default(),
        previous_proof: Some(Proof::default()),
        genesis_state: vec![1, 2, 4, 6, 7, 8, 9, 0],
    };
    let serialized = Serialize::serialize_to_vec(&proof_input);
    let deserialized: ProofInput = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof_input);

    let proof_input = ProofInput {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        block: MacroBlock::default(),
        previous_proof: None,
        genesis_state: vec![],
    };
    let serialized = Serialize::serialize_to_vec(&proof_input);
    let deserialized: ProofInput = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof_input);
}
