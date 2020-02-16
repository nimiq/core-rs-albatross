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
use algebra::bytes::FromBytes;
use groth16::{create_random_proof, prepare_verifying_key, verify_proof, Parameters, VerifyingKey};
use nimiq_bls::{KeyPair, SecureGenerate};

use nano_sync::setup::{setup_crh, CRHWindow};
use nano_sync::*;
use std::fs::File;

fn main() -> Result<(), Box<dyn Error>> {
    // This may not be cryptographically safe, use
    // `OsRng` (for example) in production software.
    let rng = &mut test_rng();

    let mut total_proving = Duration::new(0, 0);
    let mut total_verifying = Duration::new(0, 0);

    println!("Key setup");
    let num_keys = MacroBlock::SLOTS;
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
    let crh_parameters = setup_crh::<CRHWindow>();
    let min_signers = num_keys / 2;
    for i in 0..min_signers {
        macro_block1.sign(&keys[i], i, &crh_parameters);
    }

    // Reading parameters.
    println!("Reading parameters from params.bin");
    // Load parameters to file.
    let mut parameter_f = File::open("params.bin")?;
    // Verifying key.
    println!("1/11 vk");
    let alpha_g1 = FromBytes::read(&mut parameter_f)?;
    let beta_g2 = FromBytes::read(&mut parameter_f)?;
    let gamma_g2 = FromBytes::read(&mut parameter_f)?;
    let delta_g2 = FromBytes::read(&mut parameter_f)?;
    let gamma_abc_g1_len: u64 = FromBytes::read(&mut parameter_f)?;
    let mut gamma_abc_g1 = vec![];
    for _ in 0..gamma_abc_g1_len {
        gamma_abc_g1.push(FromBytes::read(&mut parameter_f)?);
    }
    let vk = VerifyingKey {
        alpha_g1,
        beta_g2,
        gamma_g2,
        delta_g2,
        gamma_abc_g1,
    };
    // Parameters.
    println!("2/11 alpha_g1");
    let alpha_g1 = FromBytes::read(&mut parameter_f)?;
    println!("3/11 beta_g1");
    let beta_g1 = FromBytes::read(&mut parameter_f)?;
    println!("4/11 beta_g2");
    let beta_g2 = FromBytes::read(&mut parameter_f)?;
    println!("5/11 delta_g1");
    let delta_g1 = FromBytes::read(&mut parameter_f)?;
    println!("6/11 delta_g2");
    let delta_g2 = FromBytes::read(&mut parameter_f)?;
    let a_query_len: u64 = FromBytes::read(&mut parameter_f)?;
    println!("7/11 a_query({})", a_query_len);
    let mut a_query = vec![];
    for _ in 0..a_query_len {
        a_query.push(FromBytes::read(&mut parameter_f)?);
    }
    let b_g1_query_len: u64 = FromBytes::read(&mut parameter_f)?;
    println!("8/11 b_g1_query({})", b_g1_query_len);
    let mut b_g1_query = vec![];
    for _ in 0..b_g1_query_len {
        b_g1_query.push(FromBytes::read(&mut parameter_f)?);
    }
    let b_g2_query_len: u64 = FromBytes::read(&mut parameter_f)?;
    println!("9/11 b_g2_query({})", b_g2_query_len);
    let mut b_g2_query = vec![];
    for _ in 0..b_g2_query_len {
        b_g2_query.push(FromBytes::read(&mut parameter_f)?);
    }
    let h_query_len: u64 = FromBytes::read(&mut parameter_f)?;
    println!("10/11 h_query({})", h_query_len);
    let mut h_query = vec![];
    for _ in 0..h_query_len {
        h_query.push(FromBytes::read(&mut parameter_f)?);
    }
    let l_query_len: u64 = FromBytes::read(&mut parameter_f)?;
    println!("11/11 l_query({})", l_query_len);
    let mut l_query = vec![];
    for _ in 0..l_query_len {
        l_query.push(FromBytes::read(&mut parameter_f)?);
    }
    drop(parameter_f);

    let params = Parameters::<SW6> {
        vk,
        alpha_g1,
        beta_g1,
        beta_g2,
        delta_g1,
        delta_g2,
        a_query,
        b_g1_query,
        b_g2_query,
        h_query,
        l_query,
    };

    let pvk = prepare_verifying_key(&params.vk);

    println!("=== Benchmarking Groth16: ====");
    println!("Proof generation");
    let start = Instant::now();
    let proof = {
        // Create an instance of our circuit (with the witness)
        let c = Circuit::new(
            1,
            genesis_keys.clone(),
            vec![macro_block1.clone()],
            crh_parameters,
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
