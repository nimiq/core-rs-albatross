use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::test_rng;
use r1cs_core::ConstraintSystem;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::mnt6_753::G1Gadget;
use r1cs_std::prelude::AllocGadget;
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::gadgets::mnt4::{PedersenCommitmentGadget, PedersenHashGadget};
use nano_sync::primitives::mnt4::{pedersen_commitment, pedersen_generators, pedersen_hash};
use nano_sync::utils::bytes_to_bits;

#[test]
fn pedersen_hash_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random bits.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    let bits = bytes_to_bits(&bytes);

    // Generate the generators for the Pedersen hash.
    let generators = pedersen_generators(256);

    // Evaluate Pedersen hash using the primitive version.
    let primitive_out = pedersen_hash(generators.clone(), bits.clone(), sum_generator_g1_mnt6());

    // Convert the result to a G1Gadget for easier comparison.
    let primitive_out_var =
        G1Gadget::alloc(cs.ns(|| "allocate primitive result"), || Ok(primitive_out)).unwrap();

    // Allocate the random bits in the circuit.
    let mut bits_var = vec![];
    for i in 0..256 {
        bits_var.push(
            Boolean::alloc(cs.ns(|| format!("allocating input bit {}", i)), || {
                Ok(&bits[i])
            })
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
        &bits_var,
        &sum_generator,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}

#[test]
fn pedersen_commitment_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random bits.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 450];
    rng.fill_bytes(&mut bytes);
    let bits = bytes_to_bits(&bytes);

    // Generate the generators for the Pedersen commitment.
    let generators = pedersen_generators(3600);

    // Evaluate Pedersen commitment using the primitive version.
    let primitive_out =
        pedersen_commitment(generators.clone(), bits.clone(), sum_generator_g1_mnt6());

    // Convert the result to a G1Gadget for easier comparison.
    let primitive_out_var =
        G1Gadget::alloc(cs.ns(|| "allocate primitive result"), || Ok(primitive_out)).unwrap();

    // Allocate the random bits in the circuit.
    let mut bits_var = vec![];
    for i in 0..3600 {
        bits_var.push(
            Boolean::alloc(cs.ns(|| format!("allocating input bit {}", i)), || {
                Ok(&bits[i])
            })
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

    // Evaluate Pedersen commitment using the gadget version.
    let gadget_out = PedersenCommitmentGadget::evaluate(
        cs.ns(|| "evaluate pedersen gadget"),
        &c_generators,
        &bits_var,
        &sum_generator,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}
