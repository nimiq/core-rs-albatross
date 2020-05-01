use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fr, G2Projective};
use algebra_core::fields::Field;
use algebra_core::{test_rng, ProjectiveCurve};
use r1cs_core::ConstraintSystem;
use r1cs_std::mnt6_753::{G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, UInt32, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::gadgets::mnt4::StateCommitmentGadget;
use nano_sync::primitives::{merkle_tree_construct, pedersen_generators, state_commitment};
use nano_sync::utils::{bytes_to_bits, serialize_g2_mnt6};

#[test]
fn state_commitment_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random inputs.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();

    let g2_point = G2Projective::prime_subgroup_generator().mul(x);
    let public_keys = vec![g2_point; 16];
    let block_number = 42;

    let tree_size = 8;

    // Evaluate state commitment using the primitive version.
    let primitive_out = state_commitment(block_number, public_keys.clone(), tree_size);

    // Convert the result to a UInt8 for easier comparison.
    let mut primitive_out_var: Vec<UInt8> = Vec::new();
    for i in 0..primitive_out.len() {
        primitive_out_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocate primitive result: byte {}", i)),
                || Ok(primitive_out[i]),
            )
            .unwrap(),
        );
    }

    // Allocate the block number in the circuit.
    let block_number_var = UInt32::alloc(cs.ns(|| "block number"), Some(block_number)).unwrap();

    // Construct the Merkle tree over the public keys.
    let mut bytes: Vec<u8> = Vec::new();
    for i in 0..public_keys.len() {
        bytes.extend_from_slice(serialize_g2_mnt6(public_keys[i]).as_ref());
    }
    let bits = bytes_to_bits(&bytes);

    let mut inputs = Vec::new();
    for i in 0..tree_size {
        inputs.push(bits[i * tree_size..(i + 1) * tree_size].to_vec());
    }
    let pks_commitment = merkle_tree_construct(inputs);

    // Allocate the public keys Merkle tree commitment in the circuit.
    let mut pks_commitment_var: Vec<UInt8> = Vec::new();
    for i in 0..primitive_out.len() {
        pks_commitment_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocate pks commitment: byte {}", i)),
                || Ok(pks_commitment[i]),
            )
            .unwrap(),
        );
    }

    // Allocate the generators
    let sum_generator = sum_generator_g1_mnt6();

    let sum_generator_var =
        G1Gadget::alloc(cs.ns(|| "sum generator"), || Ok(sum_generator)).unwrap();

    let generators = pedersen_generators(4);

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

    // Evaluate state commitment using the gadget version.
    let gadget_out = StateCommitmentGadget::evaluate(
        cs.ns(|| "evaluate state commitment gadget"),
        &block_number_var,
        &pks_commitment_var,
        &pedersen_generators_var,
        &sum_generator_var,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}
