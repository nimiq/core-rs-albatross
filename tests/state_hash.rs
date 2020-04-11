use algebra::mnt4_753::Fr as MNT4Fr;
use nimiq_bls::{KeyPair, SecureGenerate};
use r1cs_core::ConstraintSystem;
use r1cs_std::mnt6_753::G2Gadget;
use r1cs_std::prelude::{AllocGadget, UInt32};
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::{evaluate_state_hash, StateHashGadget};

#[test]
fn state_hash_test() {
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

    // Evaluate state hash using the primitive version.
    let primitive_out = evaluate_state_hash(block_number, &public_keys);

    // Convert the result to a UInt32 for easier comparison.
    let mut primitive_out_var: Vec<UInt32> = Vec::new();
    for i in 0..8 {
        primitive_out_var.push(
            UInt32::alloc(
                cs.ns(|| format!("allocate primitive result: chunk {}", i)),
                Some(primitive_out[i]),
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

    // Evaluate state hash using the gadget version.
    let gadget_out = StateHashGadget::evaluate(
        cs.ns(|| "evaluate state hash gadget"),
        &block_number_var,
        &public_keys_var,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}
