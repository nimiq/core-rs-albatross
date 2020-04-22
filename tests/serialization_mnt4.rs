use algebra::mnt4_753::{FqParameters, Fr, G1Projective, G2Projective};
use algebra::mnt6_753::Fr as MNT6Fr;
use algebra_core::fields::Field;
use algebra_core::{test_rng, ProjectiveCurve};
use r1cs_core::ConstraintSystem;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::mnt4_753::{G1Gadget, G2Gadget};
use r1cs_std::prelude::AllocGadget;
use r1cs_std::test_constraint_system::TestConstraintSystem;
use r1cs_std::ToBitsGadget;
use rand::RngCore;

use nano_sync::gadgets::mnt6::MNT6YToBitGadget;
use nano_sync::utils::{bytes_to_bits, pad_point_bits, serialize_g1_mnt4, serialize_g2_mnt4};

#[test]
fn serialization_mnt4_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT6Fr>::new();

    // Create random integer.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();

    // Create random points.
    let g2_point = G2Projective::prime_subgroup_generator().mul(x);
    let g1_point = G1Projective::prime_subgroup_generator().mul(x);

    // Allocate the random inputs in the circuit.
    let g2_point_var = G2Gadget::alloc(cs.ns(|| "alloc g1 point"), || Ok(&g2_point)).unwrap();
    let g1_point_var = G1Gadget::alloc(cs.ns(|| "alloc g2 point"), || Ok(&g1_point)).unwrap();

    // -----------  G2  -----------
    // Serialize using the primitive version.
    let bytes = serialize_g2_mnt4(g2_point);
    let bits = bytes_to_bits(&bytes);

    // Allocate the primitive result for easier comparison.
    let mut primitive_var: Vec<Boolean> = Vec::new();
    for i in 0..bits.len() {
        primitive_var.push(
            Boolean::alloc(
                cs.ns(|| format!("allocate primitive result g2: bit {}", i)),
                || Ok(bits[i]),
            )
            .unwrap(),
        );
    }

    // Serialize using the gadget version.
    let mut gadget_var = vec![];
    let x_bits = g2_point_var.x.to_bits(cs.ns(|| "g2 x to bits")).unwrap();
    let y_bit = MNT6YToBitGadget::y_to_bit_g2(cs.ns(|| "g2 y to bit"), &g2_point_var).unwrap();
    gadget_var.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));

    assert_eq!(primitive_var, gadget_var);

    // -----------  G1  -----------
    // Serialize using the primitive version.
    let bytes = serialize_g1_mnt4(g1_point);
    let bits = bytes_to_bits(&bytes);

    // Allocate the primitive result for easier comparison.
    let mut primitive_var: Vec<Boolean> = Vec::new();
    for i in 0..bits.len() {
        primitive_var.push(
            Boolean::alloc(
                cs.ns(|| format!("allocate primitive result g1: bit {}", i)),
                || Ok(bits[i]),
            )
            .unwrap(),
        );
    }

    // Serialize using the gadget version.
    let mut gadget_var = vec![];
    let x_bits = g1_point_var.x.to_bits(cs.ns(|| "g1 x to bits")).unwrap();
    let y_bit = MNT6YToBitGadget::y_to_bit_g1(cs.ns(|| "g1 y to bit"), &g1_point_var).unwrap();
    gadget_var.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));

    assert_eq!(primitive_var, gadget_var);
}
