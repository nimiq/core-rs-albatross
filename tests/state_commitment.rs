use algebra::mnt4_753::Fr as MNT4Fr;
use nimiq_bls::{KeyPair, SecureGenerate};
use r1cs_core::ConstraintSystem;
use r1cs_std::mnt6_753::{G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, UInt32, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::gadgets::mnt4::StateCommitmentGadget;
use nano_sync::primitives::mnt4::{pedersen_generators, state_commitment};

#[test]
fn state_commitment_test() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random inputs.
    let key_pair1 = KeyPair::generate_default_csprng();
    let key_pair2 = KeyPair::generate_default_csprng();
    let public_keys = vec![
        key_pair1.public_key.public_key,
        key_pair2.public_key.public_key,
    ];

    let block_number = 42;

    // Evaluate state commitment using the primitive version.
    let primitive_out = state_commitment(block_number, public_keys.clone());

    // Convert the result to a UInt8 for easier comparison.
    let mut primitive_out_var: Vec<UInt8> = Vec::new();
    for i in 0..32 {
        primitive_out_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocate primitive result: chunk {}", i)),
                || Ok(primitive_out[i]),
            )
            .unwrap(),
        );
    }

    // Allocate the random inputs in the circuit.
    let mut public_keys_var = Vec::new();
    for i in 0..2 {
        public_keys_var.push(
            G2Gadget::alloc(cs.ns(|| format!("public keys: key {}", i)), || {
                Ok(&public_keys[i])
            })
            .unwrap(),
        );
    }

    let block_number_var = UInt32::alloc(cs.ns(|| "block number"), Some(block_number)).unwrap();

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

    // Evaluate state commitment using the gadget version.
    let gadget_out = StateCommitmentGadget::evaluate(
        cs.ns(|| "evaluate state commitment gadget"),
        &block_number_var,
        &public_keys_var,
        &pedersen_generators_var,
        &sum_generator_var,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}
