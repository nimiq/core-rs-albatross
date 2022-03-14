use ark_crypto_primitives::snark::BooleanInputVar;
use ark_crypto_primitives::{CircuitSpecificSetupSNARK, SNARKGadget, SNARK};
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt4_753::constraints::{FqVar as FqVarMNT4, PairingVar};
use ark_mnt4_753::{Fq as FqMNT4, Fr as MNT4Fr, MNT4_753};
use ark_mnt6_753::constraints::FqVar as FqVarMNT6;
use ark_mnt6_753::{Fq as FqMNT6, Fr as MNT6Fr, MNT6_753};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_std::test_rng;
use rand::RngCore;

use nimiq_bls::utils::bytes_to_bits;
use nimiq_nano_zkp::utils::{pack_inputs, prepare_inputs, unpack_inputs};
use nimiq_test_log::test;

const NUMBER_OF_BITS: usize = 1024;

#[derive(Clone)]
pub struct InnerCircuit {
    // Witnesses (private)
    red_priv: Vec<bool>,
    blue_priv: Vec<bool>,
    // Inputs (public)
    red_pub: Vec<FqMNT6>,
    blue_pub: Vec<FqMNT6>,
}

impl ConstraintSynthesizer<MNT4Fr> for InnerCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Allocate all the witnesses.
        let red_priv_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.red_priv[..]))?;

        let blue_priv_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.blue_priv[..]))?;

        // Allocate all the inputs.
        let red_pub_var = Vec::<FqVarMNT6>::new_input(cs.clone(), || Ok(&self.red_pub[..]))?;

        let blue_pub_var = Vec::<FqVarMNT6>::new_input(cs, || Ok(&self.blue_pub[..]))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        let red_var_bits = unpack_inputs(red_pub_var)?[..NUMBER_OF_BITS].to_vec();

        let blue_var_bits = unpack_inputs(blue_pub_var)?[..NUMBER_OF_BITS].to_vec();

        // Compare the bits.
        red_priv_var.enforce_equal(&red_var_bits)?;

        blue_priv_var.enforce_equal(&blue_var_bits)?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct OuterCircuit {
    // Witnesses (private)
    proof: Proof<MNT4_753>,
    vk: VerifyingKey<MNT4_753>,
    // Inputs (public)
    red_pub: Vec<FqMNT4>,
    blue_pub: Vec<FqMNT4>,
}

impl ConstraintSynthesizer<MNT6Fr> for OuterCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fr>) -> Result<(), SynthesisError> {
        // Allocate the verifying key.
        let vk_var = VerifyingKeyVar::new_constant(cs.clone(), &self.vk).unwrap();

        // Allocate the proof.
        let proof_var = ProofVar::new_witness(cs.clone(), || Ok(&self.proof)).unwrap();

        // Allocate all the inputs.
        let red_pub_var = Vec::<FqVarMNT4>::new_input(cs.clone(), || Ok(&self.red_pub[..]))?;

        let blue_pub_var = Vec::<FqVarMNT4>::new_input(cs, || Ok(&self.blue_pub[..]))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        let red_var_bits = unpack_inputs(red_pub_var)?[..NUMBER_OF_BITS].to_vec();

        let blue_var_bits = unpack_inputs(blue_pub_var)?[..NUMBER_OF_BITS].to_vec();

        // Prepare inputs
        let mut proof_inputs = prepare_inputs(red_var_bits);

        proof_inputs.append(&mut prepare_inputs(blue_var_bits));

        let input_var = BooleanInputVar::new(proof_inputs);

        // Verify the ZK proof.
        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(&vk_var, &input_var, &proof_var)?
            .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}

// This test takes a very long time to finish, so run it only when necessary.
#[test]
#[ignore]
fn recursive_input_works() {
    // Create random number generator.
    let rng = &mut test_rng();

    // Create random bits and inputs for red.
    let mut bytes = [0u8; NUMBER_OF_BITS / 8];
    rng.fill_bytes(&mut bytes);

    let red_priv = bytes_to_bits(&bytes);
    let red_pub = pack_inputs(red_priv.clone());

    // Create random bits and inputs for red.
    let mut bytes = [0u8; NUMBER_OF_BITS / 8];
    rng.fill_bytes(&mut bytes);

    let blue_priv = bytes_to_bits(&bytes);
    let blue_pub = pack_inputs(blue_priv.clone());

    // Create the inner circuit.
    let circuit = InnerCircuit {
        red_priv: red_priv.clone(),
        blue_priv: blue_priv.clone(),
        red_pub: red_pub.clone(),
        blue_pub: blue_pub.clone(),
    };

    // Create the proving and verifying keys.
    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit.clone(), rng).unwrap();

    // Create the proof.
    let proof = Groth16::<MNT4_753>::prove(&pk, circuit, rng).unwrap();

    // Verify the proof.
    let mut inputs = vec![];
    inputs.extend(&red_pub);
    inputs.extend(&blue_pub);

    assert!(Groth16::<MNT4_753>::verify(&vk, &inputs, &proof).unwrap());

    // Create inputs for the outer circuit.
    let red_pub = pack_inputs(red_priv);
    let blue_pub = pack_inputs(blue_priv);

    // Create the outer circuit.
    let circuit = OuterCircuit {
        proof,
        vk,
        red_pub: red_pub.clone(),
        blue_pub: blue_pub.clone(),
    };

    // Create the proving and verifying keys.
    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit.clone(), rng).unwrap();

    // Create the proof.
    let proof = Groth16::<MNT6_753>::prove(&pk, circuit, rng).unwrap();

    // Verify the proof.
    let mut inputs = vec![];
    inputs.extend(&red_pub);
    inputs.extend(&blue_pub);

    assert!(Groth16::<MNT6_753>::verify(&vk, &inputs, &proof).unwrap());
}
