use beserial::Deserialize;
use nimiq_bls::{KeyPair, SecretKey};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::constants::{EPOCH_LENGTH, MAX_NON_SIGNERS, MIN_SIGNERS, VALIDATOR_SLOTS};
use nano_sync::primitives::evaluate_state_hash;
use nano_sync::{MacroBlock, MacroBlockCircuit};

// When running tests you are advised to set VALIDATOR_SLOTS in constants.rs to a more manageable number, for example 4.

#[test]
fn everything_works() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block with correct prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_initial_state_hash_1() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state, with the wrong block number.
    let initial_state_hash = evaluate_state_hash(previous_block_number + 1, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block with correct prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_initial_state_hash_2() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state, with the wrong public keys.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &next_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block with correct prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_final_state_hash_1() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state, with the wrong block number.
    let final_state_hash = evaluate_state_hash(next_block_number + 1, &next_keys);

    // Create macro block with correct prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_final_state_hash_2() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state, with the wrong public keys.
    let final_state_hash = evaluate_state_hash(next_block_number, &previous_keys);

    // Create macro block with correct prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn too_few_signers_prepare() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, with insufficient signers in the prepare set.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn too_few_signers_commit() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, with insufficient signers in the commit set.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_key_pair_prepare() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, with a wrong key pair in the prepare set.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }
    macro_block.sign_prepare(&key_pair2, MIN_SIGNERS - 1, previous_block_number);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_key_pair_commit() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, with a wrong key pair in the commit set.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
    }
    macro_block.sign_commit(&key_pair2, MIN_SIGNERS - 1, previous_block_number);

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

#[test]
fn wrong_signer_id_prepare() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, but we swap two values in the prepare bitmap.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }
    macro_block
        .prepare_signer_bitmap
        .swap(0, VALIDATOR_SLOTS - 1);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_signer_id_commit() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, but we swap two values in the commit bitmap.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
    }
    macro_block
        .commit_signer_bitmap
        .swap(0, VALIDATOR_SLOTS - 1);

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

#[test]
fn wrong_block_number_prepare() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, but one of the signers uses the wrong block number in the prepare round.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }
    macro_block.sign_prepare(&key_pair1, MIN_SIGNERS - 1, previous_block_number + 1);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

// TODO: Understand why this test panics.
//#[test]
fn wrong_block_number_commit() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, but one of the signers uses the wrong block number in the commit round.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS - 1 {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
    }
    macro_block.sign_commit(&key_pair1, MIN_SIGNERS - 1, previous_block_number + 1);

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

#[test]
fn mismatched_sets() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block, with mismatched prepare and commit sets. Note that not enough signers signed
    // both the prepare and commit rounds, that's why it's invalid.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in MAX_NON_SIGNERS..VALIDATOR_SLOTS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
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

#[test]
fn wrong_header_hash() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
    }

    // Change header hash of the macro block.
    macro_block.header_hash = [1; 32];

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

#[test]
fn wrong_public_keys() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
    }

    // Change public_keys of the macro block.
    macro_block.public_keys = previous_keys.clone();

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

#[test]
fn wrong_previous_keys() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys.clone());

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        next_keys,
        previous_block_number,
        macro_block,
        initial_state_hash,
        final_state_hash,
    );
    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_block_number() {
    // Create inputs.
    let (key_pair1, key_pair2) = setup_keys();
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 99;
    let next_block_number = previous_block_number + EPOCH_LENGTH;

    // Create initial state.
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let final_state_hash = evaluate_state_hash(next_block_number, &next_keys);

    // Create macro block.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..MIN_SIGNERS {
        macro_block.sign_prepare(&key_pair1, i, previous_block_number);
    }

    for i in 0..MIN_SIGNERS {
        macro_block.sign_commit(&key_pair1, i, previous_block_number);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        previous_keys,
        0,
        macro_block,
        initial_state_hash,
        final_state_hash,
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
