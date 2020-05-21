#![allow(dead_code)]

use std::{
    error::Error,
    time::{Duration, Instant},
};

// For benchmarking
use algebra::mnt4_753::{Fr as MNT4Fr, MNT4_753};
use algebra::mnt6_753::{Fr, G2Projective};
use algebra::test_rng;
use algebra_core::fields::Field;
use algebra_core::ProjectiveCurve;
use groth16::{
    create_random_proof, generate_random_parameters, prepare_verifying_key, verify_proof,
};
use r1cs_core::{ConstraintSynthesizer, ToConstraintField};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::circuits::mnt4::MacroBlockCircuit;
use nano_sync::constants::{EPOCH_LENGTH, VALIDATOR_SLOTS};
use nano_sync::primitives::{state_commitment, MacroBlock};

fn main() -> Result<(), Box<dyn Error>> {
    let mut total_setup = Duration::new(0, 0);
    let mut total_proving = Duration::new(0, 0);
    let mut total_verifying = Duration::new(0, 0);

    // Create random integers.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk_1 = Fr::from_random_bytes(&bytes).unwrap();
    rng.fill_bytes(&mut bytes[2..]);
    let sk_2 = Fr::from_random_bytes(&bytes).unwrap();

    // Create random points.
    let pk_1 = G2Projective::prime_subgroup_generator().mul(sk_1);
    let pk_2 = G2Projective::prime_subgroup_generator().mul(sk_2);

    // Create initial state.
    let previous_keys = vec![pk_1; VALIDATOR_SLOTS];
    let previous_block_number = 1;
    let initial_state_commitment = state_commitment(previous_block_number, previous_keys.clone());

    // Create final state.
    let next_keys = vec![pk_2; VALIDATOR_SLOTS];
    let next_block_number = previous_block_number + EPOCH_LENGTH;
    let final_state_commitment = state_commitment(next_block_number, next_keys.clone());

    // Create macro block with correct prepare and commit sets.
    let mut macro_block = MacroBlock::without_signatures([0; 32], next_keys);

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_prepare(sk_1.clone(), i, previous_block_number);
    }

    for i in 0..VALIDATOR_SLOTS {
        macro_block.sign_commit(sk_1, i, previous_block_number);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let c = MacroBlockCircuit::new(
        previous_keys.clone(),
        previous_block_number,
        macro_block.clone(),
        initial_state_commitment.clone(),
        final_state_commitment.clone(),
    );
    c.generate_constraints(&mut test_cs).unwrap();
    println!("Number of constraints: {}", test_cs.num_constraints());
    if !test_cs.is_satisfied() {
        println!("Unsatisfied @ {}", test_cs.which_is_unsatisfied().unwrap());
        assert!(false);
    } else {
        println!("Test passed, starting benchmark.");
    }

    // Create parameters for our circuit
    let start = Instant::now();
    let params = {
        let c = MacroBlockCircuit::new(
            previous_keys.clone(),
            previous_block_number,
            macro_block.clone(),
            initial_state_commitment.clone(),
            final_state_commitment.clone(),
        );
        generate_random_parameters::<MNT4_753, _, _>(c, rng)?
    };
    total_setup += start.elapsed();
    println!("Parameter generation finished, creating proof.");

    // Create a proof with our parameters.
    let start = Instant::now();
    let proof = {
        let c = MacroBlockCircuit::new(
            previous_keys.clone(),
            previous_block_number,
            macro_block.clone(),
            initial_state_commitment.clone(),
            final_state_commitment.clone(),
        );
        create_random_proof(c, &params, rng)?
    };
    total_proving += start.elapsed();
    println!("Proof generation finished, starting verification.");

    // Prepare inputs for verification.
    let mut inputs: Vec<MNT4Fr> = vec![];
    let field_elements: Vec<MNT4Fr> = initial_state_commitment.to_field_elements().unwrap();
    inputs.extend(field_elements);
    let field_elements: Vec<MNT4Fr> = final_state_commitment.to_field_elements().unwrap();
    inputs.extend(field_elements);
    let pvk = prepare_verifying_key(&params.vk);

    // Verify the proof
    let start = Instant::now();
    let verified = verify_proof(&pvk, &proof, &inputs).unwrap();
    total_verifying += start.elapsed();

    println!("===== Benchmarks =====");
    println!("Result: {}", verified);
    // TODO: Verify these numbers!!!
    let vk_size = 1040 + 104 * params.vk.gamma_abc_g1.len();
    let pk_size = vk_size
        + 936
        + 312 * params.b_g2_query.len()
        + 104
            * (params.a_query.len()
                + params.b_g1_query.len()
                + params.h_query.len()
                + params.l_query.len());
    println!("Verification key size: {:?} bytes", vk_size);
    println!(
        "Verification key gamma len: {:?}",
        params.vk.gamma_abc_g1.len()
    );
    println!("Prover key size: {:?} bytes", pk_size);
    println!("Average setup time: {:?} seconds", total_setup);
    println!("Average proving time: {:?} seconds", total_proving);
    println!("Average verifying time: {:?} seconds", total_verifying);

    Ok(())
}
