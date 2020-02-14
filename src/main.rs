#![allow(dead_code)]
use crate::gadgets::macro_block::MacroBlock;

mod gadgets;

// Bring in some tools for using pairing-friendly curves
// We're going to use the BLS12-377 pairing-friendly elliptic curve.
use algebra::{curves::sw6::SW6, fields::bls12_377::fq::Fq, ProjectiveCurve};
// For randomness (during paramgen and proof generation)
use algebra::test_rng;
// We're going to use the Groth 16 proving system.
use groth16::{
    create_random_proof, generate_random_parameters, prepare_verifying_key, verify_proof,
};
// For benchmarking
use std::{
    error::Error,
    time::{Duration, Instant},
};

use crate::constraints::Circuit;
use algebra::curves::bls12_377::G2Projective;
use input::Input;
use nimiq_bls::{KeyPair, SecureGenerate};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

mod constraints;
mod input;

fn main() -> Result<(), Box<dyn Error>> {
    // This may not be cryptographically safe, use
    // `OsRng` (for example) in production software.
    let rng = &mut test_rng();

    let mut total_setup = Duration::new(0, 0);
    let mut total_proving = Duration::new(0, 0);
    let mut total_verifying = Duration::new(0, 0);

    let generator = G2Projective::prime_subgroup_generator();
    let key_pair = KeyPair::generate_default_csprng();
    let key_pair2 = KeyPair::generate_default_csprng();
    let genesis_keys = vec![
        key_pair.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let mut macro_block = MacroBlock {
        header_hash: [0; 32],
        public_keys: vec![
            key_pair2.public_key.public_key,
            key_pair.public_key.public_key,
        ],
        signature: None,
        signer_bitmap: vec![],
    };

    let mut header_hash = [0; 32];
    header_hash[2] = 212;
    header_hash[20] = 118;
    let mut macro_block2 = MacroBlock {
        header_hash,
        public_keys: vec![
            key_pair.public_key.public_key,
            key_pair2.public_key.public_key,
        ],
        signature: None,
        signer_bitmap: vec![],
    };

    let last_block_public_keys = vec![
        key_pair.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let max_non_signers = 2;
    let macro_hash = macro_block.hash();
    macro_block.signature = Some(key_pair.sign_hash(macro_hash).signature);
    let signers_bitmap = vec![true, false];
    macro_block.signer_bitmap = signers_bitmap;

    let macro_hash2 = macro_block2.hash();
    macro_block2.signature = Some(key_pair2.sign_hash(macro_hash2).signature);
    let signers_bitmap = vec![true, false];
    macro_block2.signer_bitmap = signers_bitmap;

    // Test constraint system first.
    let mut test_cs = TestConstraintSystem::new();
    let c = Circuit::new(
        genesis_keys.clone(),
        vec![macro_block.clone(), macro_block2.clone()],
        generator,
        max_non_signers,
        last_block_public_keys.clone(),
    );
    c.generate_constraints(&mut test_cs)?;
    println!("Number of constraints: {}", test_cs.num_constraints());
    if !test_cs.is_satisfied() {
        println!("Unsatisfied @ {}", test_cs.which_is_unsatisfied().unwrap());
        assert!(false);
    } else {
        println!("Test passed, creating benchmark.");
    }

    // Create parameters for our circuit
    let start = Instant::now();
    let params = {
        let c = Circuit::new(
            genesis_keys.clone(),
            vec![macro_block.clone(), macro_block2.clone()],
            generator,
            max_non_signers,
            last_block_public_keys.clone(),
        );
        generate_random_parameters::<SW6, _, _>(c, rng)?
    };

    // Prepare the verification key (for proof verification)
    let pvk = prepare_verifying_key(&params.vk);
    total_setup += start.elapsed();

    // proof_vec.truncate(0);
    let start = Instant::now();
    let proof = {
        // Create an instance of our circuit (with the witness)
        let c = Circuit::new(
            genesis_keys.clone(),
            vec![macro_block.clone(), macro_block2.clone()],
            generator,
            max_non_signers,
            last_block_public_keys.clone(),
        );
        // Create a proof with our parameters.
        create_random_proof(c, &params, rng)?
    };

    total_proving += start.elapsed();

    let mut inputs: Vec<Fq> = vec![];
    for key in last_block_public_keys.iter() {
        Input::append_to_inputs(&key.into_affine(), &mut inputs);
    }

    let start = Instant::now();
    // let proof = Proof::read(&proof_vec[..]).unwrap();
    // Check the proof
    let verified = verify_proof(&pvk, &proof, &inputs).unwrap();
    total_verifying += start.elapsed();

    println!("=== Benchmarking Groth16: ====");
    println!("Result: {}", verified);
    println!(
        "Verification key size: {:?} bytes",
        336 + 48 * params.vk.gamma_abc_g1.len()
    );
    println!(
        "Verification key gamma len: {:?}",
        params.vk.gamma_abc_g1.len()
    );
    println!("Average setup time: {:?} seconds", total_setup);
    println!("Average proving time: {:?} seconds", total_proving);
    println!("Average verifying time: {:?} seconds", total_verifying);

    Ok(())
}
