use algebra::curves::bls12_377::G2Projective;
use algebra::{ProjectiveCurve, Zero};
use beserial::Deserialize;
use nano_sync::*;
use nimiq_bls::{KeyPair, SecretKey};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

#[test]
fn test_working_chain() {
    let generator = G2Projective::prime_subgroup_generator();
    let (key_pair1, key_pair2) = setup_keys();

    let genesis_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let header_hash = [
        149u8, 51u8, 189u8, 103u8, 248u8, 31u8, 60u8, 133u8, 54u8, 198u8, 186u8, 102u8, 6u8, 155u8,
        112u8, 79u8, 134u8, 244u8, 82u8, 66u8, 129u8, 80u8, 42u8, 134u8, 187u8, 98u8, 129u8, 138u8,
        181u8, 157u8, 21u8, 113u8,
    ];
    let mut macro_block1 = MacroBlock::without_signatures(
        Circuit::EPOCH_LENGTH,
        header_hash,
        vec![
            key_pair2.public_key.public_key,
            key_pair1.public_key.public_key,
        ],
    );

    let header_hash = [
        232u8, 9u8, 152u8, 215u8, 94u8, 134u8, 94u8, 176u8, 83u8, 15u8, 136u8, 16u8, 165u8, 43u8,
        103u8, 47u8, 109u8, 109u8, 187u8, 80u8, 59u8, 95u8, 87u8, 210u8, 4u8, 239u8, 209u8, 32u8,
        202u8, 102u8, 42u8, 57u8,
    ];
    let mut macro_block2 = MacroBlock::without_signatures(
        Circuit::EPOCH_LENGTH * 2,
        header_hash,
        vec![
            key_pair1.public_key.public_key,
            key_pair2.public_key.public_key,
        ],
    );

    let last_block_public_keys = macro_block2.public_keys.clone();
    // Add last public keys together.
    let mut last_block_public_key_sum = G2Projective::zero();
    for key in last_block_public_keys.iter() {
        last_block_public_key_sum += &key;
    }

    let min_signers = 1;
    macro_block1.sign(&key_pair1, 0);

    macro_block2.sign(&key_pair1, 1);
    macro_block2.sign(&key_pair2, 0);

    // Test constraint system first.
    let mut test_cs = TestConstraintSystem::new();
    let c = Circuit::new(
        3,
        genesis_keys.clone(),
        vec![macro_block1, macro_block2],
        generator,
        min_signers,
        last_block_public_key_sum,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(test_cs.is_satisfied())
}

#[test]
fn test_invalid_commit_subset() {
    let generator = G2Projective::prime_subgroup_generator();
    let (key_pair1, key_pair2) = setup_keys();

    let genesis_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let mut macro_block1 = MacroBlock::without_signatures(
        Circuit::EPOCH_LENGTH,
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
    let c = Circuit::new(
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
fn test_invalid_hash() {
    let generator = G2Projective::prime_subgroup_generator();
    let (key_pair1, key_pair2) = setup_keys();

    let genesis_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let mut macro_block1 = MacroBlock::without_signatures(
        Circuit::EPOCH_LENGTH,
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
    let c = Circuit::new(
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
        Circuit::EPOCH_LENGTH,
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
    let c = Circuit::new(
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
        Circuit::EPOCH_LENGTH,
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
    let c = Circuit::new(
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
