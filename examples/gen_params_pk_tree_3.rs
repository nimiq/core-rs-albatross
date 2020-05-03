#![allow(dead_code)]

use std::ops::Add;
use std::{
    error::Error,
    time::{Duration, Instant},
};

use algebra::mnt4_753::{Fr as MNT4Fr, MNT4_753};
use algebra::mnt6_753::{Fr, G2Projective};
use algebra::test_rng;
use algebra_core::fields::Field;
use algebra_core::{ProjectiveCurve, Zero};
use groth16::{
    create_random_proof, generate_random_parameters, prepare_verifying_key, verify_proof,
};
use r1cs_core::{ConstraintSynthesizer, ToConstraintField};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::circuits::mnt4::PKTree3Circuit;
use nano_sync::constants::{EPOCH_LENGTH, VALIDATOR_SLOTS};
use nano_sync::primitives::{merkle_tree_construct, merkle_tree_prove};
use nano_sync::utils::{byte_from_le_bits, bytes_to_bits, serialize_g2_mnt6};

fn main() -> Result<(), Box<dyn Error>> {
    let mut gen_time = Duration::new(0, 0);

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

    let path = vec![false; 3];

    let pks = [pk; VALIDATOR_SLOTS];

    let mut pks_bits = Vec::new();

    for i in 0..8 {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / 8 {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(pks[i])));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    let mut agg_pk_chunks = Vec::new();

    for i in 0..8 {
        let mut key = G2Projective::prime_subgroup_generator();

        for j in 0..VALIDATOR_SLOTS / 8 {
            if bitmap_bits[i * 8 + j] {
                key = key.add(&pks[i * 8 + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk = G2Projective::zero();

    for i in 0..8 {
        agg_pk = agg_pk.add(&agg_pk_chunks[i]);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // // Test constraint system.
    // let mut test_cs = TestConstraintSystem::new();
    // let c = PKTree3Circuit::new();
    // c.generate_constraints(&mut test_cs).unwrap();
    // println!("Number of constraints: {}", test_cs.num_constraints());
    // if !test_cs.is_satisfied() {
    //     println!("Unsatisfied @ {}", test_cs.which_is_unsatisfied().unwrap());
    //     assert!(false);
    // } else {
    //     println!("Test passed, starting benchmark.");
    // }

    // // Create parameters for our circuit
    // let start = Instant::now();
    // let params = {
    //     let PKTree3Circuit::new();
    //     generate_random_parameters::<MNT4_753, _, _>(c, rng)?
    // };
    // gen_time += start.elapsed();
    // println!("Parameter generation finished.");

    Ok(())
}
