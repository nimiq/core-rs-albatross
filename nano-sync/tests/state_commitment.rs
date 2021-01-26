use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_mnt6_753::G2Projective;
use ark_r1cs_std::prelude::{AllocVar, Boolean, UInt32};
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::ConstraintSystem;
use ark_std::{test_rng, UniformRand};

use nimiq_nano_sync::constants::VALIDATOR_SLOTS;
use nimiq_nano_sync::gadgets::mnt4::StateCommitmentGadget;
use nimiq_nano_sync::primitives::{pedersen_generators, pk_tree_construct, state_commitment};
use nimiq_nano_sync::utils::bytes_to_bits;

#[test]
fn state_commitment_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT4Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create random keys.
    let g2_point = G2Projective::rand(rng);
    let public_keys = vec![g2_point; VALIDATOR_SLOTS];

    // Create random block number.
    let block_number = u32::rand(rng);

    // Evaluate state commitment using the primitive version.
    let primitive_comm = bytes_to_bits(&state_commitment(block_number, public_keys.clone()));

    // Construct the Merkle tree over the public keys.
    let pk_tree_root = pk_tree_construct(public_keys);
    let pk_tree_root_bits = bytes_to_bits(&pk_tree_root);

    // Allocate the public key tree root in the circuit.
    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(pk_tree_root_bits)).unwrap();

    // Allocate the block number in the circuit.
    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    // Allocate the generators.
    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Evaluate state commitment using the gadget version.
    let gadget_comm = StateCommitmentGadget::evaluate(
        cs.clone(),
        &block_number_var,
        &pk_tree_root_var,
        &generators_var,
    )
    .unwrap();

    // Compare the two versions bit by bit.
    assert_eq!(primitive_comm.len(), gadget_comm.len());
    for i in 0..primitive_comm.len() {
        assert_eq!(primitive_comm[i], gadget_comm[i].value().unwrap());
    }
}
