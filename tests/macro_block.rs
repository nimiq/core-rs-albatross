use algebra::bls12_377::{G1Projective, G2Projective};
use algebra::{ProjectiveCurve, Zero};
use beserial::Deserialize;
use crypto_primitives::crh::pedersen::PedersenParameters;
use crypto_primitives::FixedLengthCRH;
use nimiq_bls::{KeyPair, PublicKey, SecretKey};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::constants::{EPOCH_LENGTH, MAX_NON_SIGNERS, MIN_SIGNERS, VALIDATOR_SLOTS};
use nano_sync::primitives::evaluate_state_hash;
use nano_sync::*;

// When running tests you are advised to set VALIDATOR_SLOTS in constants.rs to a more manageable number, for example 4.

#[test]
fn everything_works() {
    // Setup keys.
    let (key_pair1, key_pair2) = setup_keys();

    // Create initial state.
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let next_block_number = previous_block_number + EPOCH_LENGTH;
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block with correct prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_prepare(&key_pair1, i);
    }

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        previous_keys,
        previous_block_number,
        macro_block,
        initial_state_hash,
        final_state_hash,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(test_cs.is_satisfied())
}

// #[test]
fn invalid_commit_subset() {
    // Setup keys.
    let (key_pair1, key_pair2) = setup_keys();

    // Create initial state.
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let next_block_number = previous_block_number + EPOCH_LENGTH;
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block with mismatched prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i);
    }

    for i in MAX_NON_SIGNERS..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        previous_keys,
        previous_block_number,
        macro_block,
        initial_state_hash,
        final_state_hash,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

// #[test]
fn too_few_signers_prepare() {
    // Setup keys.
    let (key_pair1, key_pair2) = setup_keys();

    // Create initial state.
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let next_block_number = previous_block_number + EPOCH_LENGTH;
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block with insufficient prepare or commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_prepare(&key_pair1, i);
    }

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        previous_keys,
        previous_block_number,
        macro_block,
        initial_state_hash,
        final_state_hash,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

// #[test]
fn too_few_signers_commit() {
    // Setup keys.
    let (key_pair1, key_pair2) = setup_keys();

    // Create initial state.
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let next_block_number = previous_block_number + EPOCH_LENGTH;
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block with insufficient prepare or commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i);
    }

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_commit(&key_pair1, i);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        previous_keys,
        previous_block_number,
        macro_block,
        initial_state_hash,
        final_state_hash,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

// #[test]
// fn test_invalid_hash() {
//     let generator = G2Projective::prime_subgroup_generator();
//     let (key_pair1, key_pair2) = setup_keys();
//
//     let genesis_keys = vec![
//         key_pair1.public_key.public_key,
//         key_pair2.public_key.public_key,
//     ];
//
//     let mut macro_block1 = MacroBlock::without_signatures(
//         [0; 32],
//         vec![
//             key_pair1.public_key.public_key,
//             key_pair2.public_key.public_key,
//         ],
//     );
//
//     let last_block_public_keys = macro_block1.public_keys.clone();
//     // Add last public keys together.
//     let mut last_block_public_key_sum = G2Projective::zero();
//     for key in last_block_public_keys.iter() {
//         last_block_public_key_sum += &key;
//     }
//
//     let min_signers = 1;
//     macro_block1.sign(&key_pair1, 0);
//     macro_block1.sign(&key_pair2, 1);
//
//     macro_block1.header_hash = [1; 32];
//
//     // Test constraint system first.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         1,
//         genesis_keys.clone(),
//         vec![macro_block1],
//         generator,
//         min_signers,
//         last_block_public_key_sum,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }

//
// #[test]
// fn test_invalid_last_public_keys() {
//     let generator = G2Projective::prime_subgroup_generator();
//     let (key_pair1, key_pair2) = setup_keys();
//
//     let genesis_keys = vec![
//         key_pair1.public_key.public_key,
//         key_pair2.public_key.public_key,
//     ];
//
//     let mut macro_block1 = MacroBlock::without_signatures(
//         [0; 32],
//         vec![
//             key_pair1.public_key.public_key,
//             key_pair2.public_key.public_key,
//         ],
//     );
//
//     let last_block_public_keys = macro_block1.public_keys.clone();
//     // Add last public keys together.
//     let mut last_block_public_key_sum = G2Projective::zero();
//     for key in last_block_public_keys.iter().skip(1) {
//         last_block_public_key_sum += &key;
//     }
//
//     let min_signers = 1;
//     macro_block1.sign(&key_pair1, 0);
//
//     // Test constraint system first.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         1,
//         genesis_keys.clone(),
//         vec![macro_block1],
//         generator,
//         min_signers,
//         last_block_public_key_sum,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }

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
