use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_r1cs_std::prelude::{AllocVar, Boolean};
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::ConstraintSystem;
use ark_std::test_rng;
use rand::RngCore;

use nimiq_nano_sync::gadgets::mnt4::PedersenHashGadget;
use nimiq_nano_sync::primitives::{pedersen_generators, pedersen_hash};
use nimiq_nano_sync::utils::bytes_to_bits;

#[test]
fn pedersen_hash_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT4Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create random bytes.
    let mut bytes = [0u8; 450];
    rng.fill_bytes(&mut bytes);
    let bits = bytes_to_bits(&bytes);

    // Generate the generators for the Pedersen hash.
    let generators = pedersen_generators(6);

    // Evaluate Pedersen hash using the primitive version.
    let primitive_hash = pedersen_hash(bits.clone(), generators.clone());

    // Allocate the random bits in the circuit.
    let mut bits_var = vec![];
    for bit in bits {
        bits_var.push(Boolean::new_witness(cs.clone(), || Ok(bit)).unwrap());
    }

    // Allocate the Pedersen generators in the circuit.
    let generators_var = Vec::<G1Var>::new_witness(cs.clone(), || Ok(generators)).unwrap();

    // Evaluate Pedersen hash using the gadget version.
    let gadget_hash = PedersenHashGadget::evaluate(&bits_var, &generators_var).unwrap();

    assert_eq!(primitive_hash, gadget_hash.value().unwrap())
}
