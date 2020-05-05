#![allow(dead_code)]

use std::fs::File;
use std::ops::Add;
use std::{error::Error, time::Instant};

use algebra::mnt4_753::MNT4_753;
use algebra::mnt6_753::{Fr, G2Projective};
use algebra::test_rng;
use algebra_core::fields::Field;
use algebra_core::{ProjectiveCurve, ToBytes};
use groth16::{generate_random_parameters, Parameters};
use rand::RngCore;

use nano_sync::circuits::mnt4::PKTree5Circuit;
use nano_sync::constants::{
    sum_generator_g2_mnt6, PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS,
};
use nano_sync::primitives::{merkle_tree_construct, merkle_tree_prove};
use nano_sync::utils::{bytes_to_bits, serialize_g2_mnt6};

fn main() -> Result<(), Box<dyn Error>> {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Create parameters for our circuit
    println!("Start parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = PKTree5Circuit::new(
            pks[0..VALIDATOR_SLOTS / PK_TREE_BREADTH].to_vec(),
            pks_nodes.clone(),
            agg_pk_chunks[0],
            agg_pk_nodes.clone(),
            agg_pk_chunks[0],
            agg_pk_nodes.clone(),
            pks_commitment.clone(),
            bitmap.to_vec(),
            agg_pk_commitment.clone(),
            bitmap.to_vec(),
            agg_pk_commitment.clone(),
            position,
        );
        generate_random_parameters::<MNT4_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. Took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key to vk.bin");

    let mut file = File::create("vk.bin")?;

    ToBytes::write(&params.vk.alpha_g1, &mut file)?;
    ToBytes::write(&params.vk.beta_g2, &mut file)?;
    ToBytes::write(&params.vk.gamma_g2, &mut file)?;
    ToBytes::write(&params.vk.delta_g2, &mut file)?;
    ToBytes::write(&(params.vk.gamma_abc_g1.len() as u64), &mut file)?;
    ToBytes::write(&params.vk.gamma_abc_g1, &mut file)?;

    file.sync_all()?;

    // Proving key.
    // Save proving key to file.
    println!("Storing proving key to pk.bin");

    let mut file = File::create("pk.bin")?;

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
