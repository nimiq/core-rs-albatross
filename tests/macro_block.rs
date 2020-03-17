use algebra::bls12_377::{G1Projective, G2Projective};
use algebra::{ProjectiveCurve, Zero};
use beserial::Deserialize;
use crypto_primitives::crh::pedersen::PedersenParameters;
use crypto_primitives::FixedLengthCRH;
use nimiq_bls::{KeyPair, PublicKey, SecretKey};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::*;

#[test]
fn test_invalid_commit_subset() {
    let generator = G2Projective::prime_subgroup_generator();
    let (key_pair1, key_pair2) = setup_keys();

    let genesis_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let mut macro_block1 = MacroBlock::without_signatures(
        [0; 32],
        vec![
            key_pair1.public_key.public_key,
            key_pair2.public_key.public_key,
        ],
    );

    let last_block_public_keys = macro_block1.public_keys.clone();
    // Add last public keys together.
    let mut last_block_public_key_sum = G2Projective::zero();
    for key in last_block_public_keys.iter() {
        last_block_public_key_sum += &key;
    }

    let min_signers = 1;
    macro_block1.sign_prepare(&key_pair1, 0);
    macro_block1.sign_commit(&key_pair2, 1);

    // Test constraint system first.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        genesis_keys.clone(),
        1,
        vec![macro_block1],
        generator,
        min_signers,
        last_block_public_key_sum,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn test_invalid_hash() {
    let generator = G2Projective::prime_subgroup_generator();
    let (key_pair1, key_pair2) = setup_keys();

    let genesis_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let mut macro_block1 = MacroBlock::without_signatures(
        [0; 32],
        vec![
            key_pair1.public_key.public_key,
            key_pair2.public_key.public_key,
        ],
    );

    let last_block_public_keys = macro_block1.public_keys.clone();
    // Add last public keys together.
    let mut last_block_public_key_sum = G2Projective::zero();
    for key in last_block_public_keys.iter() {
        last_block_public_key_sum += &key;
    }

    let min_signers = 1;
    macro_block1.sign(&key_pair1, 0);
    macro_block1.sign(&key_pair2, 1);

    macro_block1.header_hash = [1; 32];

    // Test constraint system first.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        1,
        genesis_keys.clone(),
        vec![macro_block1],
        generator,
        min_signers,
        last_block_public_key_sum,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn test_too_few_signers() {
    let generator = G2Projective::prime_subgroup_generator();
    let (key_pair1, key_pair2) = setup_keys();

    let genesis_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let mut macro_block1 = MacroBlock::without_signatures(
        [0; 32],
        vec![
            key_pair1.public_key.public_key,
            key_pair2.public_key.public_key,
        ],
    );

    let last_block_public_keys = macro_block1.public_keys.clone();
    // Add last public keys together.
    let mut last_block_public_key_sum = G2Projective::zero();
    for key in last_block_public_keys.iter() {
        last_block_public_key_sum += &key;
    }

    let min_signers = 2;
    macro_block1.sign(&key_pair1, 0);

    // Test constraint system first.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        1,
        genesis_keys.clone(),
        vec![macro_block1],
        generator,
        min_signers,
        last_block_public_key_sum,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn test_invalid_last_public_keys() {
    let generator = G2Projective::prime_subgroup_generator();
    let (key_pair1, key_pair2) = setup_keys();

    let genesis_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let mut macro_block1 = MacroBlock::without_signatures(
        [0; 32],
        vec![
            key_pair1.public_key.public_key,
            key_pair2.public_key.public_key,
        ],
    );

    let last_block_public_keys = macro_block1.public_keys.clone();
    // Add last public keys together.
    let mut last_block_public_key_sum = G2Projective::zero();
    for key in last_block_public_keys.iter().skip(1) {
        last_block_public_key_sum += &key;
    }

    let min_signers = 1;
    macro_block1.sign(&key_pair1, 0);

    // Test constraint system first.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        1,
        genesis_keys.clone(),
        vec![macro_block1],
        generator,
        min_signers,
        last_block_public_key_sum,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

fn setup_keys() -> (KeyPair, KeyPair) {
    let key1 = KeyPair::from(
        SecretKey::deserialize_from_vec(
            &hex::decode("589c058ba169884a35d6fc2d4b4a3f1ec789791e7df7912d5865a996882da303")
                .unwrap(),
        )
        .unwrap(),
    );
    let key2 = KeyPair::from(
        SecretKey::deserialize_from_vec(
            &hex::decode("4f5757d23f9a9677cbffac2db6c7c7b2fa9b93056732e516eae6c21a08a56a0f")
                .unwrap(),
        )
        .unwrap(),
    );
    (key1, key2)
}
