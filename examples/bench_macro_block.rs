#![allow(dead_code)]

// For benchmarking
use std::{
    error::Error,
    time::{Duration, Instant},
};

use algebra::sw6::{Fr as SW6Fr, SW6};
use algebra::test_rng;
use groth16::{
    create_random_proof, generate_random_parameters, prepare_verifying_key, verify_proof,
};
use nimiq_bls::{KeyPair, SecureGenerate};
use r1cs_core::{ConstraintSynthesizer, ToConstraintField};
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::constants::{EPOCH_LENGTH, VALIDATOR_SLOTS};
use nano_sync::*;

fn main() -> Result<(), Box<dyn Error>> {
    // This may not be cryptographically safe, use
    // `OsRng` (for example) in production software.
    //
    let rng = &mut test_rng();

    let mut total_setup = Duration::new(0, 0);
    let mut total_proving = Duration::new(0, 0);
    let mut total_verifying = Duration::new(0, 0);

    // Setup keys.
    let key_pair1 = KeyPair::generate_default_csprng();
    let key_pair2 = KeyPair::generate_default_csprng();

    // Create initial state.
    let previous_keys = vec![key_pair1.public_key.public_key; VALIDATOR_SLOTS];
    let previous_block_number = 1;
    let initial_state_hash = evaluate_state_hash(previous_block_number, &previous_keys);

    // Create final state.
    let next_keys = vec![key_pair2.public_key.public_key; VALIDATOR_SLOTS];
    let next_block_number = previous_block_number + EPOCH_LENGTH;
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
        previous_keys.clone(),
        previous_block_number,
        macro_block.clone(),
        initial_state_hash.clone(),
        final_state_hash.clone(),
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
            initial_state_hash.clone(),
            final_state_hash.clone(),
        );
        generate_random_parameters::<SW6, _, _>(c, rng)?
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
            initial_state_hash.clone(),
            final_state_hash.clone(),
        );
        create_random_proof(c, &params, rng)?
    };
    total_proving += start.elapsed();
    println!("Proof generation finished, starting verification.");

    // // Prepare inputs for verification.
    let mut inputs: Vec<SW6Fr> = vec![];
    let field_elements: Vec<SW6Fr> = initial_state_hash.to_field_elements().unwrap();
    inputs.extend(field_elements);
    let field_elements: Vec<SW6Fr> = final_state_hash.to_field_elements().unwrap();
    inputs.extend(field_elements);
    let pvk = prepare_verifying_key(&params.vk);

    // // Verify the proof
    let start = Instant::now();
    let verified = verify_proof(&pvk, &proof, &inputs).unwrap();
    total_verifying += start.elapsed();

    println!("===== Benchmarks =====");
    println!("Result: {}", verified);
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
