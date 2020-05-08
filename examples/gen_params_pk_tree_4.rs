#![allow(dead_code)]

use std::fs::File;
use std::{error::Error, time::Instant};

use algebra::mnt6_753::MNT6_753;
use algebra_core::{test_rng, ProjectiveCurve, ToBytes};
use groth16::{generate_random_parameters, Parameters, Proof};
use rand::RngCore;

use nano_sync::circuits::mnt6::PKTree4Circuit;
use nano_sync::constants::VALIDATOR_SLOTS;
use nano_sync::utils::{gen_rand_g1_mnt4, gen_rand_g2_mnt4};

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

    // Create dummy inputs.
    let left_proof = Proof {
        a: gen_rand_g1_mnt4().into_affine(),
        b: gen_rand_g2_mnt4().into_affine(),
        c: gen_rand_g1_mnt4().into_affine(),
    };

    let right_proof = Proof {
        a: gen_rand_g1_mnt4().into_affine(),
        b: gen_rand_g2_mnt4().into_affine(),
        c: gen_rand_g1_mnt4().into_affine(),
    };

    let mut pks_commitment = [0u8; 95];
    rng.fill_bytes(&mut pks_commitment);

    let mut prepare_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut prepare_signer_bitmap);

    let mut left_prepare_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut left_prepare_agg_pk_commitment);

    let mut right_prepare_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut right_prepare_agg_pk_commitment);

    let mut commit_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut commit_signer_bitmap);

    let mut left_commit_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut left_commit_agg_pk_commitment);

    let mut right_commit_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut right_commit_agg_pk_commitment);

    let position = 0;

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT6_753> = {
        let c = PKTree4Circuit::new(
            left_proof,
            right_proof,
            pks_commitment.to_vec(),
            prepare_signer_bitmap.to_vec(),
            left_prepare_agg_pk_commitment.to_vec(),
            right_prepare_agg_pk_commitment.to_vec(),
            commit_signer_bitmap.to_vec(),
            left_commit_agg_pk_commitment.to_vec(),
            right_commit_agg_pk_commitment.to_vec(),
            position,
        );
        generate_random_parameters::<MNT6_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. Took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/pk_tree_4.bin")?;

    ToBytes::write(&params.vk.alpha_g1, &mut file)?;
    ToBytes::write(&params.vk.beta_g2, &mut file)?;
    ToBytes::write(&params.vk.gamma_g2, &mut file)?;
    ToBytes::write(&params.vk.delta_g2, &mut file)?;
    ToBytes::write(&(params.vk.gamma_abc_g1.len() as u64), &mut file)?;
    ToBytes::write(&params.vk.gamma_abc_g1, &mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/pk_tree_4.bin")?;

    ToBytes::write(&params.vk.alpha_g1, &mut file)?;
    ToBytes::write(&params.vk.beta_g2, &mut file)?;
    ToBytes::write(&params.vk.gamma_g2, &mut file)?;
    ToBytes::write(&params.vk.delta_g2, &mut file)?;
    ToBytes::write(&(params.vk.gamma_abc_g1.len() as u64), &mut file)?;
    ToBytes::write(&params.vk.gamma_abc_g1, &mut file)?;
    ToBytes::write(&params.beta_g1, &mut file)?;
    ToBytes::write(&params.delta_g1, &mut file)?;
    ToBytes::write(&(params.a_query.len() as u64), &mut file)?;
    ToBytes::write(&params.a_query, &mut file)?;
    ToBytes::write(&(params.b_g1_query.len() as u64), &mut file)?;
    ToBytes::write(&params.b_g1_query, &mut file)?;
    ToBytes::write(&(params.b_g2_query.len() as u64), &mut file)?;
    ToBytes::write(&params.b_g2_query, &mut file)?;
    ToBytes::write(&(params.h_query.len() as u64), &mut file)?;
    ToBytes::write(&params.h_query, &mut file)?;
    ToBytes::write(&(params.l_query.len() as u64), &mut file)?;
    ToBytes::write(&params.l_query, &mut file)?;

    file.sync_all()?;

    Ok(())
}
