use ark_mnt4_753::constraints::{G1Var, G2Var};
use ark_mnt4_753::{G1Projective, G2Projective};
use ark_mnt6_753::Fr as MNT6Fr;
use ark_r1cs_std::prelude::AllocVar;
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::ConstraintSystem;
use ark_std::{test_rng, UniformRand};

use nimiq_nano_sync::gadgets::mnt6::SerializeGadget;
use nimiq_nano_sync::primitives::{serialize_g1_mnt4, serialize_g2_mnt4};
use nimiq_nano_sync::utils::bytes_to_bits;

#[test]
fn serialization_g1_mnt4_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT6Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create random point.
    let g1_point = G1Projective::rand(rng);

    // Allocate the random inputs in the circuit.
    let g1_point_var = G1Var::new_witness(cs.clone(), || Ok(g1_point.clone())).unwrap();

    // Serialize using the primitive version.
    let primitive_bytes = serialize_g1_mnt4(g1_point);
    let primitive_bits = bytes_to_bits(&primitive_bytes);

    // Serialize using the gadget version.
    let gadget_bits = SerializeGadget::serialize_g1(cs.clone(), &g1_point_var).unwrap();

    // Compare the two versions bit by bit.
    assert_eq!(primitive_bits.len(), gadget_bits.len());
    for i in 0..primitive_bits.len() {
        assert_eq!(primitive_bits[i], gadget_bits[i].value().unwrap());
    }
}

#[test]
fn serialization_g2_mnt4_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT6Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create random point.
    let g2_point = G2Projective::rand(rng);

    // Allocate the random inputs in the circuit.
    let g2_point_var = G2Var::new_witness(cs.clone(), || Ok(g2_point.clone())).unwrap();

    // Serialize using the primitive version.
    let primitive_bytes = serialize_g2_mnt4(g2_point);
    let primitive_bits = bytes_to_bits(&primitive_bytes);

    // Serialize using the gadget version.
    let gadget_bits = SerializeGadget::serialize_g2(cs.clone(), &g2_point_var).unwrap();

    // Compare the two versions bit by bit.
    assert_eq!(primitive_bits.len(), gadget_bits.len());
    for i in 0..primitive_bits.len() {
        assert_eq!(primitive_bits[i], gadget_bits[i].value().unwrap());
    }
}
