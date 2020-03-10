#![allow(dead_code)]

use std::{
    error::Error,
    time::{Duration, Instant},
};
// For benchmarking
use std::fs::File;

use algebra::{curves::sw6::SW6, Zero};
// We're going to use the Groth 16 proving system.
use algebra::bytes::ToBytes;
// Bring in some tools for using pairing-friendly curves
// We're going to use the BLS12-377 pairing-friendly elliptic curve.
use algebra::curves::bls12_377::G2Projective;
// For randomness (during paramgen and proof generation)
use algebra::test_rng;
use groth16::generate_random_parameters;
use nimiq_bls::{KeyPair, SecureGenerate};

use nano_sync::constants::{setup_crh, CRHWindow};
use nano_sync::*;

fn main() -> Result<(), Box<dyn Error>> {
    // This may not be cryptographically safe, use
    // `OsRng` (for example) in production software.
    let rng = &mut test_rng();

    let mut total_setup = Duration::new(0, 0);

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

    println!("=== Benchmarking Groth16: ====");
    println!("Parameter generation");
    // Create parameters for our circuit
    let start = Instant::now();
    let params = {
        let c = Circuit::new(
            1,
            genesis_keys.clone(),
            vec![macro_block1.clone()],
            crh_parameters,
            min_signers,
            last_block_public_key_sum,
        );
        generate_random_parameters::<SW6, _, _>(c, rng)?
    };
    total_setup += start.elapsed();
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

    // Save parameters to file.
    println!("Storing parameters to params.bin");
    //    pub struct Parameters<E: PairingEngine> {
    //        pub vk:         VerifyingKey<E>,
    //        pub alpha_g1:   E::G1Affine,
    //        pub beta_g1:    E::G1Affine,
    //        pub beta_g2:    E::G2Affine,
    //        pub delta_g1:   E::G1Affine,
    //        pub delta_g2:   E::G2Affine,
    //        pub a_query:    Vec<E::G1Affine>,
    //        pub b_g1_query: Vec<E::G1Affine>,
    //        pub b_g2_query: Vec<E::G2Affine>,
    //        pub h_query:    Vec<E::G1Affine>,
    //        pub l_query:    Vec<E::G1Affine>,
    //    }
    let mut parameter_f = File::create("params.bin")?;
    // Verifying key.
    println!("1/11: Verifying key");
    ToBytes::write(&params.vk.alpha_g1, &mut parameter_f)?;
    ToBytes::write(&params.vk.beta_g2, &mut parameter_f)?;
    ToBytes::write(&params.vk.gamma_g2, &mut parameter_f)?;
    ToBytes::write(&params.vk.delta_g2, &mut parameter_f)?;
    ToBytes::write(&(params.vk.gamma_abc_g1.len() as u64), &mut parameter_f)?;
    ToBytes::write(&params.vk.gamma_abc_g1, &mut parameter_f)?;
    // Parameters.
    println!("2/11: alpha_g1");
    ToBytes::write(&params.alpha_g1, &mut parameter_f)?;
    println!("3/11: beta_g1");
    ToBytes::write(&params.beta_g1, &mut parameter_f)?;
    println!("4/11: beta_g2");
    ToBytes::write(&params.beta_g2, &mut parameter_f)?;
    println!("5/11: delta_g1");
    ToBytes::write(&params.delta_g1, &mut parameter_f)?;
    println!("6/11: delta_g2");
    ToBytes::write(&params.delta_g2, &mut parameter_f)?;
    println!("7/11: a_query({})", params.a_query.len());
    ToBytes::write(&(params.a_query.len() as u64), &mut parameter_f)?;
    ToBytes::write(&params.a_query, &mut parameter_f)?;
    println!("8/11: b_g1_query({})", params.b_g1_query.len());
    ToBytes::write(&(params.b_g1_query.len() as u64), &mut parameter_f)?;
    ToBytes::write(&params.b_g1_query, &mut parameter_f)?;
    println!("9/11: b_g2_query({})", params.b_g2_query.len());
    ToBytes::write(&(params.b_g2_query.len() as u64), &mut parameter_f)?;
    ToBytes::write(&params.b_g2_query, &mut parameter_f)?;
    println!("10/11: h_query({})", params.h_query.len());
    ToBytes::write(&(params.h_query.len() as u64), &mut parameter_f)?;
    ToBytes::write(&params.h_query, &mut parameter_f)?;
    println!("11/11: l_query({})", params.l_query.len());
    ToBytes::write(&(params.l_query.len() as u64), &mut parameter_f)?;
    ToBytes::write(&params.l_query, &mut parameter_f)?;
    parameter_f.sync_all()?;

    Ok(())
}
