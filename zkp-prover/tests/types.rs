use ark_groth16::Proof;
use beserial::{Deserialize, Serialize};
use nimiq_database::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_primitives::policy;
use nimiq_zkp_prover::types::ZKProof;

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
fn it_serializes_and_deserializes_zkp_proof() {
    let proof = ZKProof {
        block_number: policy::BLOCKS_PER_EPOCH,
        proof: Some(Proof::default()),
    };
    let serialized = Serialize::serialize_to_vec(&proof);
    let deserialized: ZKProof = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof);

    let proof = ZKProof {
        block_number: 0,
        proof: None,
    };
    let serialized = Serialize::serialize_to_vec(&proof);
    let deserialized: ZKProof = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, proof);
}
