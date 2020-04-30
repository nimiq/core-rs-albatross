use std::ops::AddAssign;

use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fq, Fr, G2Projective};
use algebra_core::fields::Field;
use algebra_core::{test_rng, ProjectiveCurve, Zero};
use r1cs_core::ConstraintSystem;
use r1cs_std::mnt6_753::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::constants::{sum_generator_g1_mnt6, MAX_NON_SIGNERS, VALIDATOR_SLOTS};
use nano_sync::gadgets::mnt4::MacroBlockGadget;
use nano_sync::primitives::{pedersen_generators, state_commitment, MacroBlock};

// When running tests you are advised to set VALIDATOR_SLOTS in constants.rs to a more manageable number, for example 8.

//#[test]
fn macro_block_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random points.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk_1 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_1 = G2Projective::prime_subgroup_generator().mul(sk_1);
    rng.fill_bytes(&mut bytes[2..]);
    let sk_2 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_2 = G2Projective::prime_subgroup_generator().mul(sk_2);

    // Create more block parameters.
    let next_keys = vec![pk_2; VALIDATOR_SLOTS];
    let block_number = 77;
    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();
    let final_state_commitment = state_commitment(block_number, next_keys, 8);

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..VALIDATOR_SLOTS {
        block.sign_prepare(sk_1, i, final_state_commitment.clone());
        prepare_agg_pk.add_assign(&pk_1);
    }

    for i in 0..VALIDATOR_SLOTS {
        block.sign_commit(sk_1, i, final_state_commitment.clone());
        commit_agg_pk.add_assign(&pk_1);
    }

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::alloc(cs.ns(|| "alloc macro block"), || Ok(&block)).unwrap();

    let mut final_state_commitment_var = Vec::new();
    for i in 0..final_state_commitment.len() {
        final_state_commitment_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocating final state commitment: byte {}", i)),
                || Ok(&final_state_commitment[i]),
            )
            .unwrap(),
        );
    }

    let prepare_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating prepare agg pk"), || Ok(prepare_agg_pk)).unwrap();

    let commit_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating commit agg pk"), || Ok(commit_agg_pk)).unwrap();

    let max_non_signers_var = FqGadget::alloc(cs.ns(|| "alloc max non signers"), || {
        Ok(Fq::from(MAX_NON_SIGNERS as u64))
    })
    .unwrap();

    let sig_generator_var = G2Gadget::alloc(cs.ns(|| "alloc signature generator"), || {
        Ok(G2Projective::prime_subgroup_generator())
    })
    .unwrap();

    let sum_generator_g1_var = G1Gadget::alloc(cs.ns(|| "alloc sum generator g1"), || {
        Ok(sum_generator_g1_mnt6())
    })
    .unwrap();

    let pedersen_generators = pedersen_generators(256);
    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..256 {
        pedersen_generators_var.push(
            G1Gadget::alloc(
                cs.ns(|| format!("alloc pedersen_generators: generator {}", i)),
                || Ok(pedersen_generators[i]),
            )
            .unwrap(),
        );
    }

    // Verify Macro Block.
    let result = block_var.verify(
        cs.ns(|| "verify macro block"),
        &final_state_commitment_var,
        &prepare_agg_pk_var,
        &commit_agg_pk_var,
        &max_non_signers_var,
        &sig_generator_var,
        &sum_generator_g1_var,
        &pedersen_generators_var,
    );

    assert!(result.is_ok())
}

//#[test]
fn wrong_final_state_commitment() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random points.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk_1 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_1 = G2Projective::prime_subgroup_generator().mul(sk_1);
    rng.fill_bytes(&mut bytes[2..]);
    let sk_2 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_2 = G2Projective::prime_subgroup_generator().mul(sk_2);

    // Create more block parameters.
    let next_keys = vec![pk_2; VALIDATOR_SLOTS];
    let block_number = 77;
    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();
    let final_state_commitment = state_commitment(block_number, next_keys, 8);

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..VALIDATOR_SLOTS {
        block.sign_prepare(sk_1, i, final_state_commitment.clone());
        prepare_agg_pk.add_assign(&pk_1);
    }

    for i in 0..VALIDATOR_SLOTS {
        block.sign_commit(sk_1, i, final_state_commitment.clone());
        commit_agg_pk.add_assign(&pk_1);
    }

    // Allocate wrong final state commitment.
    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);

    let mut final_state_commitment_var = Vec::new();
    for i in 0..bytes.len() {
        final_state_commitment_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocating final state commitment: byte {}", i)),
                || Ok(&bytes[i]),
            )
            .unwrap(),
        );
    }

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::alloc(cs.ns(|| "alloc macro block"), || Ok(&block)).unwrap();

    let prepare_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating prepare agg pk"), || Ok(prepare_agg_pk)).unwrap();

    let commit_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating commit agg pk"), || Ok(commit_agg_pk)).unwrap();

    let max_non_signers_var = FqGadget::alloc(cs.ns(|| "alloc max non signers"), || {
        Ok(Fq::from(MAX_NON_SIGNERS as u64))
    })
    .unwrap();

    let sig_generator_var = G2Gadget::alloc(cs.ns(|| "alloc signature generator"), || {
        Ok(G2Projective::prime_subgroup_generator())
    })
    .unwrap();

    let sum_generator_g1_var = G1Gadget::alloc(cs.ns(|| "alloc sum generator g1"), || {
        Ok(sum_generator_g1_mnt6())
    })
    .unwrap();

    let pedersen_generators = pedersen_generators(256);
    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..256 {
        pedersen_generators_var.push(
            G1Gadget::alloc(
                cs.ns(|| format!("alloc pedersen_generators: generator {}", i)),
                || Ok(pedersen_generators[i]),
            )
            .unwrap(),
        );
    }

    // Verify Macro Block.
    let result = block_var.verify(
        cs.ns(|| "verify macro block"),
        &final_state_commitment_var,
        &prepare_agg_pk_var,
        &commit_agg_pk_var,
        &max_non_signers_var,
        &sig_generator_var,
        &sum_generator_g1_var,
        &pedersen_generators_var,
    );

    assert!(result.is_err())
}

//#[test]
fn wrong_prepare_agg_pk() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random points.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk_1 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_1 = G2Projective::prime_subgroup_generator().mul(sk_1);
    rng.fill_bytes(&mut bytes[2..]);
    let sk_2 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_2 = G2Projective::prime_subgroup_generator().mul(sk_2);

    // Create more block parameters.
    let next_keys = vec![pk_2; VALIDATOR_SLOTS];
    let block_number = 77;
    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();
    let final_state_commitment = state_commitment(block_number, next_keys, 8);

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..VALIDATOR_SLOTS {
        block.sign_prepare(sk_1, i, final_state_commitment.clone());
        prepare_agg_pk.add_assign(&pk_1);
    }

    for i in 0..VALIDATOR_SLOTS {
        block.sign_commit(sk_1, i, final_state_commitment.clone());
        commit_agg_pk.add_assign(&pk_1);
    }

    // Allocate wrong prepare_agg_pk
    let prepare_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating prepare agg pk"), || Ok(pk_2)).unwrap();

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::alloc(cs.ns(|| "alloc macro block"), || Ok(&block)).unwrap();

    let mut final_state_commitment_var = Vec::new();
    for i in 0..final_state_commitment.len() {
        final_state_commitment_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocating final state commitment: byte {}", i)),
                || Ok(&final_state_commitment[i]),
            )
            .unwrap(),
        );
    }

    let commit_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating commit agg pk"), || Ok(commit_agg_pk)).unwrap();

    let max_non_signers_var = FqGadget::alloc(cs.ns(|| "alloc max non signers"), || {
        Ok(Fq::from(MAX_NON_SIGNERS as u64))
    })
    .unwrap();

    let sig_generator_var = G2Gadget::alloc(cs.ns(|| "alloc signature generator"), || {
        Ok(G2Projective::prime_subgroup_generator())
    })
    .unwrap();

    let sum_generator_g1_var = G1Gadget::alloc(cs.ns(|| "alloc sum generator g1"), || {
        Ok(sum_generator_g1_mnt6())
    })
    .unwrap();

    let pedersen_generators = pedersen_generators(256);
    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..256 {
        pedersen_generators_var.push(
            G1Gadget::alloc(
                cs.ns(|| format!("alloc pedersen_generators: generator {}", i)),
                || Ok(pedersen_generators[i]),
            )
            .unwrap(),
        );
    }

    // Verify Macro Block.
    let result = block_var.verify(
        cs.ns(|| "verify macro block"),
        &final_state_commitment_var,
        &prepare_agg_pk_var,
        &commit_agg_pk_var,
        &max_non_signers_var,
        &sig_generator_var,
        &sum_generator_g1_var,
        &pedersen_generators_var,
    );

    assert!(result.is_err())
}

#[test]
fn wrong_commit_agg_pk() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random points.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk_1 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_1 = G2Projective::prime_subgroup_generator().mul(sk_1);
    rng.fill_bytes(&mut bytes[2..]);
    let sk_2 = Fr::from_random_bytes(&bytes).unwrap();
    let pk_2 = G2Projective::prime_subgroup_generator().mul(sk_2);

    // Create more block parameters.
    let next_keys = vec![pk_2; VALIDATOR_SLOTS];
    let block_number = 77;
    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();
    let final_state_commitment = state_commitment(block_number, next_keys, 8);

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..VALIDATOR_SLOTS {
        block.sign_prepare(sk_1, i, final_state_commitment.clone());
        prepare_agg_pk.add_assign(&pk_1);
    }

    for i in 0..VALIDATOR_SLOTS {
        block.sign_commit(sk_1, i, final_state_commitment.clone());
        commit_agg_pk.add_assign(&pk_1);
    }

    // Allocate wrong prepare_agg_pk
    let commit_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating commit agg pk"), || Ok(pk_2)).unwrap();

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::alloc(cs.ns(|| "alloc macro block"), || Ok(&block)).unwrap();

    let mut final_state_commitment_var = Vec::new();
    for i in 0..final_state_commitment.len() {
        final_state_commitment_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocating final state commitment: byte {}", i)),
                || Ok(&final_state_commitment[i]),
            )
            .unwrap(),
        );
    }

    let prepare_agg_pk_var =
        G2Gadget::alloc(cs.ns(|| "allocating prepare agg pk"), || Ok(prepare_agg_pk)).unwrap();

    let max_non_signers_var = FqGadget::alloc(cs.ns(|| "alloc max non signers"), || {
        Ok(Fq::from(MAX_NON_SIGNERS as u64))
    })
    .unwrap();

    let sig_generator_var = G2Gadget::alloc(cs.ns(|| "alloc signature generator"), || {
        Ok(G2Projective::prime_subgroup_generator())
    })
    .unwrap();

    let sum_generator_g1_var = G1Gadget::alloc(cs.ns(|| "alloc sum generator g1"), || {
        Ok(sum_generator_g1_mnt6())
    })
    .unwrap();

    let pedersen_generators = pedersen_generators(256);
    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..256 {
        pedersen_generators_var.push(
            G1Gadget::alloc(
                cs.ns(|| format!("alloc pedersen_generators: generator {}", i)),
                || Ok(pedersen_generators[i]),
            )
            .unwrap(),
        );
    }

    // Verify Macro Block.
    let result = block_var.verify(
        cs.ns(|| "verify macro block"),
        &final_state_commitment_var,
        &prepare_agg_pk_var,
        &commit_agg_pk_var,
        &max_non_signers_var,
        &sig_generator_var,
        &sum_generator_g1_var,
        &pedersen_generators_var,
    );

    assert!(result.is_err())
}

// //#[test]
// fn too_few_signers_prepare() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, with insufficient signers in the prepare set.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS - 1 {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn too_few_signers_commit() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, with insufficient signers in the commit set.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS - 1 {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_key_pair_prepare() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, with a wrong key pair in the prepare set.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS - 1 {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//     macro_block.sign_prepare(&key_pair2, MIN_SIGNERS - 1, previous_block_number);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_key_pair_commit() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, with a wrong key pair in the commit set.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS - 1 {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//     macro_block.sign_commit(&key_pair2, MIN_SIGNERS - 1, previous_block_number);
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_signer_id_prepare() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, but we swap two values in the prepare bitmap.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//     macro_block
//         .prepare_signer_bitmap
//         .swap(0, VALIDATOR_SLOTS - 1);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_signer_id_commit() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, but we swap two values in the commit bitmap.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS - 1 {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//     macro_block
//         .commit_signer_bitmap
//         .swap(0, VALIDATOR_SLOTS - 1);
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_block_number_prepare() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, but one of the signers uses the wrong block number in the prepare round.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS - 1 {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//     macro_block.sign_prepare(&key_pair1, MIN_SIGNERS - 1, previous_block_number + 1);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_block_number_commit() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, but one of the signers uses the wrong block number in the commit round.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS - 1 {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//     macro_block.sign_commit(&key_pair1, MIN_SIGNERS - 1, previous_block_number + 1);
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn mismatched_sets() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block, with mismatched prepare and commit sets. Note that not enough signers signed
//     // both the prepare and commit rounds, that's why it's invalid.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in MAX_NON_SIGNERS..VALIDATOR_SLOTS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_header_hash() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Change header hash of the macro block.
//     macro_block.header_hash = [1; 32];
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_public_keys() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Change public_keys of the macro block.
//     macro_block.public_keys = previous_keys.clone();
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_previous_keys() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys.clone());
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         next_keys,
//         previous_block_number,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// //#[test]
// fn wrong_block_number() {
//     // Create inputs.
//     let (key_pair1, key_pair2) = setup_keys();
//     let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
//     let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
//     let previous_block_number = 99;
//     let next_block_number = previous_block_number + EPOCH_LENGTH;
//
//     // Create initial state.
//     let initial_state_commitment = state_commitment(previous_block_number, &previous_keys);
//
//     // Create final state.
//     let final_state_commitment = state_commitment(next_block_number, &next_keys);
//
//     // Create macro block.
//     let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_prepare(&key_pair1, i, previous_block_number);
//     }
//
//     for i in 0..MIN_SIGNERS {
//         macro_block.sign_commit(&key_pair1, i, previous_block_number);
//     }
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let c = MacroBlockCircuit::new(
//         previous_keys,
//         0,
//         macro_block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//     c.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied())
// }
//
// fn setup_keys() -> (KeyPair, KeyPair) {
//     let key1 = KeyPair::from(
//         SecretKey::deserialize_from_vec(
//             &hex::decode("589c058ba169884a35d6fc2d4b4a3f1ec789791e7df7912d5865a996882da303")
//                 .unwrap(),
//         )
//         .unwrap(),
//     );
//     let key2 = KeyPair::from(
//         SecretKey::deserialize_from_vec(
//             &hex::decode("4f5757d23f9a9677cbffac2db6c7c7b2fa9b93056732e516eae6c21a08a56a0f")
//                 .unwrap(),
//         )
//         .unwrap(),
//     );
//     (key1, key2)
// }
