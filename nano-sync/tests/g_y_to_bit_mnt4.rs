use ark_mnt4_753::constraints::{G1Var, G2Var};
use ark_mnt4_753::{G1Projective, G2Projective};
use ark_mnt6_753::Fr as MNT6Fr;
use ark_r1cs_std::prelude::AllocVar;
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::ConstraintSystem;
use ark_std::{test_rng, UniformRand};
use nimiq_nano_sync::gadgets::mnt6::YToBitGadget;
use nimiq_nano_sync::primitives::{serialize_g1_mnt4, serialize_g2_mnt4};
use nimiq_nano_sync::utils::bytes_to_bits;

#[test]
fn y_to_bit_g1_mnt4_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT6Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    for _ in 0..10 {
        // Create random point.
        let g1_point = G1Projective::rand(rng);

        // Allocate the random inputs in the circuit and convert it to affine form.
        let g1_point_var = G1Var::new_witness(cs.clone(), || Ok(g1_point.clone()))
            .unwrap()
            .to_affine()
            .unwrap();

        // Serialize using the primitive version and get the first bit (which is the y flag).
        let bytes = serialize_g1_mnt4(g1_point);
        let bits = bytes_to_bits(&bytes);
        let primitive_y_bit = bits[0];

        // Serialize using the gadget version and get the boolean value.
        let gadget_y_bit = YToBitGadget::y_to_bit_g1(cs.clone(), &g1_point_var)
            .unwrap()
            .value()
            .unwrap();

        assert_eq!(primitive_y_bit, gadget_y_bit);
    }
}

#[test]
fn y_to_bit_g2_mnt4_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT6Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    for _ in 0..10 {
        // Create random point.
        let g2_point = G2Projective::rand(rng);

        // Allocate the random inputs in the circuit and convert it to affine form.
        let g2_point_var = G2Var::new_witness(cs.clone(), || Ok(g2_point.clone()))
            .unwrap()
            .to_affine()
            .unwrap();

        // Serialize using the primitive version and get the first bit (which is the y flag).
        let bytes = serialize_g2_mnt4(g2_point);
        let bits = bytes_to_bits(&bytes);
        let primitive_y_bit = bits[0];

        // Serialize using the gadget version and get the boolean value.
        let gadget_y_bit = YToBitGadget::y_to_bit_g2(cs.clone(), &g2_point_var)
            .unwrap()
            .value()
            .unwrap();

        assert_eq!(primitive_y_bit, gadget_y_bit);
    }
}
