#![allow(dead_code)]

use std::error::Error;

use algebra::mnt6_753::{Fr, G1Affine, G1Projective, G2Affine, G2Projective, MNT6_753};
use algebra_core::fields::Field;
use algebra_core::{test_rng, ProjectiveCurve};
use groth16::{Proof, VerifyingKey};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_counter::ConstraintCounter;
use rand::RngCore;

use nano_sync::circuits::mnt4::MergerCircuit;

fn main() -> Result<(), Box<dyn Error>> {
    let proof = Proof::<MNT6_753> {
        a: generate_random_g1(),
        b: generate_random_g2(),
        c: generate_random_g1(),
    };

    let vk_merger = VerifyingKey::<MNT6_753> {
        alpha_g1: generate_random_g1(),
        beta_g2: generate_random_g2(),
        gamma_g2: generate_random_g2(),
        delta_g2: generate_random_g2(),
        gamma_abc_g1: vec![generate_random_g1(); 6],
    };

    let vk_block = VerifyingKey::<MNT6_753> {
        alpha_g1: generate_random_g1(),
        beta_g2: generate_random_g2(),
        gamma_g2: generate_random_g2(),
        delta_g2: generate_random_g2(),
        gamma_abc_g1: vec![generate_random_g1(); 4],
    };

    let c = MergerCircuit::new(
        proof.clone(),
        proof,
        vk_merger,
        vk_block,
        vec![1u8; 95],
        false,
        vec![1u8; 95],
        vec![1u8; 95],
        vec![1u8; 95],
    );

    let mut counter = ConstraintCounter::new();

    c.generate_constraints(&mut counter).unwrap();

    println!("Number of constraints: {}", counter.num_constraints());

    Ok(())
}

pub fn generate_random_g1() -> G1Affine {
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    G1Projective::prime_subgroup_generator()
        .mul(x)
        .into_affine()
}

pub fn generate_random_g2() -> G2Affine {
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    G2Projective::prime_subgroup_generator()
        .mul(x)
        .into_affine()
}
