use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::test_rng;
use r1cs_core::ConstraintSystem;
use r1cs_std::mnt6_753::G1Gadget;
use r1cs_std::prelude::{AllocGadget, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::gadgets::PedersenHashGadget;
use nano_sync::primitives::{evaluate_pedersen, setup_pedersen};

#[test]
fn pedersen_test() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random bytes.
    let rng = &mut test_rng();
    let mut input = [0u8; 32];
    rng.fill_bytes(&mut input);

    // Generate the generators for the Pedersen hash.
    let generators = setup_pedersen();

    // Evaluate Pedersen hash using the primitive version.
    let primitive_out =
        evaluate_pedersen(generators.clone(), input.to_vec(), sum_generator_g1_mnt6());

    // Convert the result to a G1Gadget for easier comparison.
    let primitive_out_var =
        G1Gadget::alloc(cs.ns(|| "allocate primitive result"), || Ok(primitive_out)).unwrap();

    // Allocate the random bytes in the circuit.
    let mut input_bytes = vec![];
    for (byte_i, input_byte) in input.iter().enumerate() {
        input_bytes.push(
            UInt8::alloc(
                cs.ns(|| format!("allocating input byte {}", byte_i)),
                || Ok(*input_byte),
            )
            .unwrap(),
        );
    }

    // Allocate the Pedersen generators in the circuit.
    let mut c_generators = Vec::new();
    for i in 0..generators.len() {
        let base = G1Gadget::alloc(
            &mut cs.ns(|| format!("allocating pedersen generator {}", i)),
            || Ok(&generators[i]),
        )
        .unwrap();
        c_generators.push(base);
    }

    // Allocate the sum generator in the circuit.
    let sum_generator = G1Gadget::alloc(cs.ns(|| "allocating sum generator"), || {
        Ok(sum_generator_g1_mnt6())
    })
    .unwrap();

    // Evaluate Pedersen hash using the gadget version.
    let gadget_out = PedersenHashGadget::evaluate(
        cs.ns(|| "evaluate pedersen gadget"),
        &c_generators,
        &input_bytes,
        &sum_generator,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}
