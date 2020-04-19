use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::FqParameters;
use nimiq_bls::{KeyPair, SecureGenerate};
use r1cs_core::ConstraintSystem;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::mnt6_753::G2Gadget;
use r1cs_std::prelude::AllocGadget;
use r1cs_std::test_constraint_system::TestConstraintSystem;
use r1cs_std::ToBitsGadget;

use nano_sync::gadgets::mnt4::YToBitGadget;
use nano_sync::utils::{bytes_to_bits, pad_point_bits, serialize_g2_mnt6};

#[test]
fn serialization_mnt4_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random inputs.
    let key_pair = KeyPair::generate_default_csprng();
    let pk = key_pair.public_key.public_key;
    let sk = key_pair.secret_key.secret_key;

    // Allocate the random inputs in the circuit.
    let pk_var = G2Gadget::alloc(cs.ns(|| "alloc pk"), || Ok(&pk)).unwrap();

    // -----------  G2  -----------
    // Serialize using the primitive version.
    let bytes = serialize_g2_mnt6(pk);
    let bits = bytes_to_bits(&bytes);

    // Allocate the primitive result for easier comparison.
    let mut primitive_var: Vec<Boolean> = Vec::new();
    for i in 0..bits.len() {
        primitive_var.push(
            Boolean::alloc(
                cs.ns(|| format!("allocate primitive result: bit {}", i)),
                || Ok(bits[i]),
            )
            .unwrap(),
        );
    }
    println!(
        "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
        primitive_var[0],
        primitive_var[1],
        primitive_var[2],
        primitive_var[3],
        primitive_var[4],
        primitive_var[5],
        primitive_var[6],
        primitive_var[7]
    );

    // Serialize using the gadget version.
    let mut gadget_var = vec![];
    let x_bits = pk_var.x.to_bits(cs.ns(|| "x to bits")).unwrap();
    let y_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| "y to bit"), &pk_var).unwrap();
    gadget_var.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));
    println!(
        "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
        gadget_var[0],
        gadget_var[1],
        gadget_var[2],
        gadget_var[3],
        gadget_var[4],
        gadget_var[5],
        gadget_var[6],
        gadget_var[7]
    );

    //assert_eq!(primitive_var, gadget_var);
    assert_eq!(primitive_var.len(), gadget_var.len());
    assert!(false);
}
