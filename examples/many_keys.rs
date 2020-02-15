#![allow(dead_code)]

// For benchmarking
use std::{
    error::Error,
    time::{Duration, Instant},
};

// Bring in some tools for using pairing-friendly curves
// We're going to use the BLS12-377 pairing-friendly elliptic curve.
use algebra::curves::bls12_377::G2Projective;
use algebra::{curves::sw6::SW6, fields::bls12_377::fq::Fq, ProjectiveCurve, Zero};
// For randomness (during paramgen and proof generation)
use algebra::test_rng;
// We're going to use the Groth 16 proving system.
use groth16::{
    create_random_proof, generate_random_parameters, prepare_verifying_key, verify_proof,
};
use nimiq_bls::{KeyPair, SecureGenerate};

use nano_sync::*;

fn main() -> Result<(), Box<dyn Error>> {
    // This may not be cryptographically safe, use
    // `OsRng` (for example) in production software.
    let rng = &mut test_rng();

    let mut total_setup = Duration::new(0, 0);
    let mut total_proving = Duration::new(0, 0);
    let mut total_verifying = Duration::new(0, 0);

    println!("Key setup");
    let num_keys = MacroBlock::SLOTS;
    let generator = G2Projective::prime_subgroup_generator();
    let mut keys = vec![];
    for _ in 0..num_keys {
        let key_pair = KeyPair::generate_default_csprng();
        keys.push(key_pair);
    }
    let genesis_keys: Vec<G2Projective> =
        keys.iter().map(|key| key.public_key.public_key).collect();

    println!("Macro block generation");
    let mut macro_block1 =
        MacroBlock::without_signatures(Circuit::EPOCH_LENGTH, [0; 32], genesis_keys.clone());

    let last_block_public_keys = genesis_keys.clone();
    // Add last public keys together.
    let mut last_block_public_key_sum = G2Projective::zero();
    for key in last_block_public_keys.iter() {
        last_block_public_key_sum += &key;
    }

    println!("Macro block signing");
    let min_signers = num_keys / 2;
    for i in 0..min_signers {
        macro_block1.sign(&keys[i], i);
    }

    println!("=== Benchmarking Groth16: ====");
    println!("Parameter generation");
    // Create parameters for our circuit
    let start = Instant::now();
    let params = {
        let c = Circuit::new(
            1,
            genesis_keys.clone(),
            vec![macro_block1.clone()],
            generator,
            min_signers,
            last_block_public_key_sum,
        );
        generate_random_parameters::<SW6, _, _>(c, rng)?
    };

    // Prepare the verification key (for proof verification)
    let pvk = prepare_verifying_key(&params.vk);
    total_setup += start.elapsed();
    println!(
        "Verification key size: {:?} bytes",
        336 + 48 * params.vk.gamma_abc_g1.len()
    );
    println!(
        "Verification key gamma len: {:?}",
        params.vk.gamma_abc_g1.len()
    );
    println!("Average setup time: {:?} seconds", total_setup);

    println!("Proof generation");
    let start = Instant::now();
    let proof = {
        // Create an instance of our circuit (with the witness)
        let c = Circuit::new(
            1,
            genesis_keys.clone(),
            vec![macro_block1.clone()],
            generator,
            min_signers,
            last_block_public_key_sum,
        );
        // Create a proof with our parameters.
        create_random_proof(c, &params, rng)?
    };

    total_proving += start.elapsed();
    println!("Average proving time: {:?} seconds", total_proving);

    let mut inputs: Vec<Fq> = vec![];
    Input::append_to_inputs(&last_block_public_key_sum.into_affine(), &mut inputs);

    println!("Proof verification");
    let start = Instant::now();
    // let proof = Proof::read(&proof_vec[..]).unwrap();
    // Check the proof
    let verified = verify_proof(&pvk, &proof, &inputs).unwrap();
    total_verifying += start.elapsed();

    println!("Result: {}", verified);
    println!("Average verifying time: {:?} seconds", total_verifying);

    Ok(())
}
