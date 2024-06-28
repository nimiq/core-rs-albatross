use std::path::PathBuf;

use ark_groth16::Proof;
use nimiq_block::MacroBlock;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_utils::zkp_test_data::ZKP_TEST_KEYS_PATH;
use nimiq_zkp_component::types::{ProofInput, ZKPState, ZKProof};

#[test]
fn it_serializes_and_deserializes_zk_proof() {
    let b = ZKProof {
        block_number: 0,
        proof: None,
    };
    let serialized = Serialize::serialize_to_vec(&b);
    let deserialized = ZKProof::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, b);

    let proof = ZKProof {
        block_number: 0,
        proof: Some(Proof::default()),
    };
    let serialized = Serialize::serialize_to_vec(&proof);
    let deserialized = ZKProof::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof);
}

#[test]
fn it_serializes_and_deserializes_to_bytes_zk_proof() {
    let proof = ZKProof {
        block_number: 0,
        proof: None,
    };
    let serialized = proof.as_value_bytes();
    let deserialized: ZKProof = FromDatabaseBytes::from_value_bytes(&serialized);
    assert_eq!(deserialized, proof);

    let proof = ZKProof {
        block_number: 0,
        proof: Some(Proof::default()),
    };
    let serialized = proof.as_value_bytes();
    let deserialized: ZKProof = FromDatabaseBytes::from_value_bytes(&serialized);
    assert_eq!(deserialized, proof);
}

#[test]
fn it_serializes_and_deserializes_zkp_state() {
    let state = ZKPState {
        latest_block: MacroBlock::default(),
        latest_proof: Some(Proof::default()),
    };
    let serialized = Serialize::serialize_to_vec(&state);
    let deserialized = ZKPState::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, state);

    let state = ZKPState {
        latest_block: MacroBlock::default(),
        latest_proof: None,
    };
    let serialized = Serialize::serialize_to_vec(&state);
    let deserialized = ZKPState::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, state);
}

#[test]
fn it_serializes_and_deserializes_proof_input() {
    let proof_input = ProofInput {
        previous_block: MacroBlock::default(),
        previous_proof: Some(Proof::default()),
        final_block: MacroBlock::default(),
        genesis_header_hash: [2; 32],
        prover_keys_path: PathBuf::from(ZKP_TEST_KEYS_PATH),
    };
    let serialized = Serialize::serialize_to_vec(&proof_input);
    let deserialized = ProofInput::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof_input);

    let proof_input = ProofInput {
        previous_block: MacroBlock::default(),
        previous_proof: None,
        final_block: MacroBlock::default(),
        genesis_header_hash: [0; 32],
        prover_keys_path: PathBuf::from(ZKP_TEST_KEYS_PATH),
    };
    let serialized = Serialize::serialize_to_vec(&proof_input);
    let deserialized = ProofInput::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof_input);
}
