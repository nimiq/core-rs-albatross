use ark_groth16::Proof;
use beserial::{Deserialize, Serialize};
use nimiq_database::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;
use nimiq_zkp_prover::types::{ZKPState, ZKProof};

#[test]
fn it_serializes_and_deserializes_zkp_state() {
    let b = ZKPState {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: 0,
        latest_proof: None,
    };
    let serialized = Serialize::serialize_to_vec(&b);
    let deserialized: ZKPState = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, b);

    let c = ZKPState {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: 0,
        latest_proof: Some(Proof::default()),
    };
    let serialized = Serialize::serialize_to_vec(&c);
    let deserialized: ZKPState = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, c);
}

#[test]
fn it_serializes_and_deserializes_to_bytes_zkp_state() {
    let b = ZKPState {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: 0,
        latest_proof: None,
    };
    let serialized = b.as_database_bytes();
    let deserialized: ZKPState = FromDatabaseValue::copy_from_database(&serialized).unwrap();
    assert_eq!(deserialized, b);

    let c = ZKPState {
        latest_pks: vec![],
        latest_header_hash: Blake2bHash::default(),
        latest_block_number: 0,
        latest_proof: Some(Proof::default()),
    };
    let serialized = c.as_database_bytes();
    let deserialized: ZKPState = FromDatabaseValue::copy_from_database(&serialized).unwrap();
    assert_eq!(deserialized, c);
}

#[test]
fn it_serializes_and_deserializes_zkp_proof() {
    let b = ZKProof {
        block_number: policy::BLOCKS_PER_EPOCH,
        proof: Some(Proof::default()),
    };
    let serialized = Serialize::serialize_to_vec(&b);
    let deserialized: ZKProof = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, b);

    let c = ZKProof {
        block_number: 0,
        proof: None,
    };
    let serialized = Serialize::serialize_to_vec(&c);
    let deserialized: ZKProof = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, c);
}
