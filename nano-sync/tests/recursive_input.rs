use ark_crypto_primitives::snark::BooleanInputVar;
use ark_crypto_primitives::{CircuitSpecificSetupSNARK, SNARKGadget, SNARK};
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::Groth16;
use ark_mnt4_753::constraints::PairingVar;
use ark_mnt4_753::{Fr as MNT4Fr, MNT4_753};
use ark_mnt6_753::constraints::FqVar as FqMNT6;
use ark_mnt6_753::{Fq, Fr as MNT6Fr};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::{
    ConstraintSynthesizer, ConstraintSystem, ConstraintSystemRef, SynthesisError,
};
use ark_std::test_rng;
use rand::RngCore;

use nimiq_nano_sync::utils::{bytes_to_bits, pack_inputs, prepare_inputs, unpack_inputs};

const NUMBER_OF_BITS: usize = 1024;

#[derive(Clone)]
pub struct TestCircuit {
    // Witnesses (private)
    red_priv: Vec<bool>,
    blue_priv: Vec<bool>,
    // Inputs (public)
    red_pub: Vec<Fq>,
    blue_pub: Vec<Fq>,
}

impl ConstraintSynthesizer<MNT4Fr> for TestCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Allocate all the witnesses.
        let red_priv_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.red_priv[..]))?;

        let blue_priv_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.blue_priv[..]))?;

        // Allocate all the inputs.
        let red_pub_var = Vec::<FqMNT6>::new_input(cs.clone(), || Ok(&self.red_pub[..]))?;

        let blue_pub_var = Vec::<FqMNT6>::new_input(cs.clone(), || Ok(&self.blue_pub[..]))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        let red_var_bits = unpack_inputs(red_pub_var)?[..NUMBER_OF_BITS].to_vec();

        let blue_var_bits = unpack_inputs(blue_pub_var)?[..NUMBER_OF_BITS].to_vec();

        // Compare the bits.
        red_priv_var.enforce_equal(&red_var_bits)?;

        blue_priv_var.enforce_equal(&blue_var_bits)?;

        Ok(())
    }
}

#[test]
fn input_works() {
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

    // Create the circuit.
    let circuit = TestCircuit {
        red_priv: red_priv.clone(),
        blue_priv: blue_priv.clone(),
        red_pub: red_pub.clone(),
        blue_pub: blue_pub.clone(),
    };

    // Create the proving and verifying keys.
    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit.clone(), rng).unwrap();

    // Check the constraint system.
    {
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        circuit.clone().generate_constraints(cs.clone()).unwrap();

        match cs.which_is_unsatisfied().unwrap() {
            None => {}
            Some(s) => {
                println!("Unsatisfied @ {}", s);
                assert!(false);
            }
        }
    }

    // Create the proof.
    let proof = Groth16::<MNT4_753>::prove(&pk, circuit, rng).unwrap();

    // Verify the proof.
    let mut inputs = vec![];
    inputs.extend(&red_pub);
    inputs.extend(&blue_pub);

    assert!(Groth16::<MNT4_753>::verify(&vk, &inputs, &proof).unwrap());

    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT6Fr>::new_ref();

    // Allocate the verifying key.
    let vk_var = VerifyingKeyVar::new_witness(cs.clone(), || Ok(vk)).unwrap();

    // Allocate the proof.
    let proof_var = ProofVar::new_witness(cs.clone(), || Ok(proof)).unwrap();

    // Allocate the inputs.
    let red_priv_var = Vec::<Boolean<MNT6Fr>>::new_input(cs.clone(), || Ok(red_priv)).unwrap();

    let blue_priv_var = Vec::<Boolean<MNT6Fr>>::new_input(cs.clone(), || Ok(blue_priv)).unwrap();

    // Prepare inputs
    let mut proof_inputs = prepare_inputs(red_priv_var);

    proof_inputs.append(&mut prepare_inputs(blue_priv_var));

    let input_var = BooleanInputVar::new(proof_inputs);

    // Verify the ZK proof.
    assert!(
        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(&vk_var, &input_var, &proof_var)
            .unwrap()
            .value()
            .unwrap()
    )
}
