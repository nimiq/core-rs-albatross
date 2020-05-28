use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fq, Fr, G1Projective, G2Projective};
use algebra::MNT6_753;
use algebra_core::fields::Field;
use algebra_core::{test_rng, ProjectiveCurve};
use crypto_primitives::nizk::groth16::constraints::VerifyingKeyGadget;
use groth16::VerifyingKey;
use r1cs_core::ConstraintSystem;
use r1cs_std::mnt6_753::{G1Gadget, PairingGadget};
use r1cs_std::prelude::{AllocGadget, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::gadgets::mnt4::VKCommitmentGadget;
use nano_sync::primitives::{pedersen_generators, vk_commitment};

// When running tests you are advised to run only one test at a time or you might run out of RAM.
// Also they take a long time to run. This is why they have the ignore flag.

#[test]
#[ignore]
fn vk_commitment_test() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random input.
    let mut vk: VerifyingKey<MNT6_753> = VerifyingKey::default();
    vk.alpha_g1 = G1Projective::prime_subgroup_generator()
        .mul(generate_random_int())
        .into_affine();
    vk.beta_g2 = G2Projective::prime_subgroup_generator()
        .mul(generate_random_int())
        .into_affine();
    vk.gamma_g2 = G2Projective::prime_subgroup_generator()
        .mul(generate_random_int())
        .into_affine();
    vk.delta_g2 = G2Projective::prime_subgroup_generator()
        .mul(generate_random_int())
        .into_affine();
    vk.gamma_abc_g1 = vec![
        G1Projective::prime_subgroup_generator()
            .mul(generate_random_int())
            .into_affine(),
        G1Projective::prime_subgroup_generator()
            .mul(generate_random_int())
            .into_affine(),
    ];

    // Allocate the random input in the circuit.
    let vk_var: VerifyingKeyGadget<MNT6_753, Fq, PairingGadget> =
        VerifyingKeyGadget::alloc(cs.ns(|| "alloc vk"), || Ok(vk.clone())).unwrap();

    // Allocate the generators
    let sum_generator = sum_generator_g1_mnt6();

    let sum_generator_var =
        G1Gadget::alloc(cs.ns(|| "sum generator"), || Ok(sum_generator)).unwrap();

    let generators = pedersen_generators(256);

    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..generators.len() {
        pedersen_generators_var.push(
            G1Gadget::alloc(
                cs.ns(|| format!("pedersen_generators: generator {}", i)),
                || Ok(generators[i]),
            )
            .unwrap(),
        );
    }

    // Evaluate state commitment using the primitive version.
    let primitive_out = vk_commitment(vk);

    // Convert the result to a UInt8 for easier comparison.
    let mut primitive_out_var: Vec<UInt8> = Vec::new();
    for i in 0..primitive_out.len() {
        primitive_out_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocate primitive result: chunk {}", i)),
                || Ok(primitive_out[i]),
            )
            .unwrap(),
        );
    }

    // Evaluate state commitment using the gadget version.
    let gadget_out = VKCommitmentGadget::evaluate(
        cs.ns(|| "evaluate vk commitment gadget"),
        &vk_var,
        &pedersen_generators_var,
        &sum_generator_var,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}

pub fn generate_random_int() -> Fr {
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    Fr::from_random_bytes(&bytes).unwrap()
}
