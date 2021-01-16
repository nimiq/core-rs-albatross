use ark_ec::ProjectiveCurve;
use ark_groth16::constraints::VerifyingKeyVar;
use ark_groth16::VerifyingKey;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, PairingVar};
use ark_mnt6_753::MNT6_753;
use ark_mnt6_753::{G1Projective, G2Projective};
use ark_r1cs_std::prelude::AllocVar;
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::ConstraintSystem;
use ark_std::{test_rng, UniformRand};
use nimiq_bls::utils::bytes_to_bits;
use nimiq_nano_sync::gadgets::mnt4::VKCommitmentGadget;
use nimiq_nano_sync::primitives::{pedersen_generators, vk_commitment};

#[ignore] // TODO: remove.
#[test]
fn vk_commitment_test() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT4Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create verifying key.
    let mut vk = VerifyingKey::<MNT6_753>::default();
    vk.alpha_g1 = G1Projective::rand(rng).into_affine();
    vk.beta_g2 = G2Projective::rand(rng).into_affine();
    vk.gamma_g2 = G2Projective::rand(rng).into_affine();
    vk.delta_g2 = G2Projective::rand(rng).into_affine();
    vk.gamma_abc_g1 = vec![
        G1Projective::rand(rng).into_affine(),
        G1Projective::rand(rng).into_affine(),
    ];

    // Evaluate vk commitment using the primitive version.
    let primitive_comm = bytes_to_bits(&vk_commitment(vk.clone()));

    // Allocate the verifying key in the circuit.
    let vk_var =
        VerifyingKeyVar::<_, PairingVar>::new_witness(cs.clone(), || Ok(vk.clone())).unwrap();

    // Allocate the generators.
    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(14))).unwrap();

    // Evaluate vk commitment using the gadget version.
    let gadget_comm = VKCommitmentGadget::evaluate(cs.clone(), &vk_var, &generators_var).unwrap();

    // Compare the two versions bit by bit.
    assert_eq!(primitive_comm.len(), gadget_comm.len());
    for i in 0..primitive_comm.len() {
        assert_eq!(primitive_comm[i], gadget_comm[i].value().unwrap());
    }
}
