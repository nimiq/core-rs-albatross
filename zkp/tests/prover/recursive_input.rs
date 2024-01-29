use ark_crypto_primitives::snark::{CircuitSpecificSetupSNARK, SNARKGadget, SNARK};
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Groth16, Proof, VerifyingKey,
};
use ark_mnt4_753::{Fq as FqMNT4, MNT4_753};
use ark_mnt6_753::{constraints::PairingVar, Fq as FqMNT6, MNT6_753};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{
    ConstraintSynthesizer, ConstraintSystemRef, SynthesisError, ToConstraintField,
};
use ark_std::rand::RngCore;
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_zkp_circuits::recursive::RecursiveInputVar;

const NUMBER_OF_BYTES: usize = 128;

#[derive(Clone)]
pub struct InnerCircuit {
    // Witnesses (private)
    red_priv: Vec<u8>,
    blue_priv: Vec<u8>,
    // Inputs (public)
    red_pub: Vec<u8>,
    blue_pub: Vec<u8>,
}

impl ConstraintSynthesizer<FqMNT4> for InnerCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<FqMNT4>) -> Result<(), SynthesisError> {
        // Allocate all the witnesses.
        let red_priv_var = UInt8::<FqMNT4>::new_witness_vec(cs.clone(), &self.red_priv[..])?;

        let blue_priv_var = UInt8::<FqMNT4>::new_witness_vec(cs.clone(), &self.blue_priv[..])?;

        // Allocate all the inputs.
        let red_pub_var = UInt8::<FqMNT4>::new_input_vec(cs.clone(), &self.red_pub[..])?;

        let blue_pub_var = UInt8::<FqMNT4>::new_input_vec(cs, &self.blue_pub[..])?;

        // Compare the bytes.
        red_priv_var.enforce_equal(&red_pub_var)?;

        blue_priv_var.enforce_equal(&blue_pub_var)?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct OuterCircuit {
    // Witnesses (private)
    proof: Proof<MNT6_753>,
    vk: VerifyingKey<MNT6_753>,
    // Inputs (public)
    red_pub: Vec<u8>,
    blue_pub: Vec<u8>,
}

impl ConstraintSynthesizer<FqMNT6> for OuterCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<FqMNT6>) -> Result<(), SynthesisError> {
        // Allocate the verifying key.
        let vk_var = VerifyingKeyVar::new_constant(cs.clone(), &self.vk).unwrap();

        // Allocate the proof.
        let proof_var = ProofVar::new_witness(cs.clone(), || Ok(&self.proof)).unwrap();

        // Allocate all the inputs.
        let red_pub_var = UInt8::<FqMNT6>::new_input_vec(cs.clone(), &self.red_pub[..])?;

        let blue_pub_var = UInt8::<FqMNT6>::new_input_vec(cs, &self.blue_pub[..])?;

        // Prepare inputs
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&red_pub_var)?;
        proof_inputs.push(&blue_pub_var)?;

        // Verify the ZK proof.
        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_var,
            &proof_inputs.into(),
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}

// This test takes a very long time to finish, so run it only when necessary.
#[test]
#[cfg_attr(not(feature = "expensive-tests"), ignore)]
fn recursive_input_works() {
    // Create random number generator.
    let rng = &mut test_rng(false);

    // Create random bits and inputs for red.
    let mut bytes = [0u8; NUMBER_OF_BYTES];
    rng.fill_bytes(&mut bytes);

    let red = bytes.to_vec();

    // Create random bits and inputs for red.
    let mut bytes = [0u8; NUMBER_OF_BYTES];
    rng.fill_bytes(&mut bytes);

    let blue = bytes.to_vec();

    // Create the inner circuit.
    let circuit = InnerCircuit {
        red_priv: red.clone(),
        blue_priv: blue.clone(),
        red_pub: red.clone(),
        blue_pub: blue.clone(),
    };

    // Create the proving and verifying keys.
    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit.clone(), rng).unwrap();

    // Create the proof.
    let proof = Groth16::<MNT6_753>::prove(&pk, circuit, rng).unwrap();

    // Verify the proof.
    let mut inputs = vec![];
    inputs.extend(&red.to_field_elements().unwrap());
    inputs.extend(&blue.to_field_elements().unwrap());

    assert!(Groth16::<MNT6_753>::verify(&vk, &inputs, &proof).unwrap());

    // Create the outer circuit.
    let circuit = OuterCircuit {
        proof,
        vk,
        red_pub: red.clone(),
        blue_pub: blue.clone(),
    };

    // Create the proving and verifying keys.
    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit.clone(), rng).unwrap();

    // Create the proof.
    let proof = Groth16::<MNT4_753>::prove(&pk, circuit, rng).unwrap();

    // Verify the proof.
    let mut inputs = vec![];
    inputs.extend(&red.to_field_elements().unwrap());
    inputs.extend(&blue.to_field_elements().unwrap());

    assert!(Groth16::<MNT4_753>::verify(&vk, &inputs, &proof).unwrap());
}
