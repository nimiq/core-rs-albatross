use ark_crypto_primitives::snark::BooleanInputVar;
use ark_crypto_primitives::{CircuitSpecificSetupSNARK, SNARKGadget, SNARK};
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::Groth16;
use ark_mnt4_753::constraints::PairingVar;
use ark_mnt4_753::{Fr as MNT4Fr, MNT4_753};
use ark_mnt6_753::constraints::FqVar;
use ark_mnt6_753::{Fq, Fr as MNT6Fr};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::{
    ConstraintSynthesizer, ConstraintSystem, ConstraintSystemRef, SynthesisError,
};
use ark_std::test_rng;
use rand::RngCore;

use nimiq_nano_sync::utils::{bytes_to_bits, pack_inputs, prepare_inputs, unpack_inputs};

#[derive(Clone)]
pub struct TestCircuit {
    // Witness (private)
    hash_priv: Vec<bool>,
    // Input (public)
    hash_pub: Vec<Fq>,
}

impl ConstraintSynthesizer<MNT4Fr> for TestCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Allocate all the witnesses.
        let hash_priv_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.hash_priv[..]))?;

        // Allocate all the inputs.
        let hash_pub_var = Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.hash_pub[..]))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        let hash_pub_bits = unpack_inputs(hash_pub_var)?[..1024].to_vec();

        // Compare the two.
        hash_priv_var.enforce_equal(&hash_pub_bits)
    }
}

#[test]
fn input_works() {
    // Create random number generator.
    let rng = &mut test_rng();

    // Create random bits.
    let mut bytes = [0u8; 128];
    rng.fill_bytes(&mut bytes);

    let hash_priv = bytes_to_bits(&bytes);

    let hash_pub = prepare_inputs(hash_priv.clone());

    // Create the circuit.
    let circuit = TestCircuit {
        hash_priv: hash_priv.clone(),
        hash_pub,
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

    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT6Fr>::new_ref();

    // Allocate the random bits in the circuit.
    let bits_var = Vec::<Boolean<MNT6Fr>>::new_witness(cs.clone(), || Ok(hash_priv)).unwrap();

    // Allocate the verifying key.
    let vk_var = VerifyingKeyVar::new_witness(cs.clone(), || Ok(vk)).unwrap();

    // Allocate the proof.
    let proof_var = ProofVar::new_witness(cs.clone(), || Ok(proof)).unwrap();

    // Verify the ZK proof.
    let proof_inputs = pack_inputs(bits_var);

    let input_var = BooleanInputVar::new(proof_inputs);

    assert!(
        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(&vk_var, &input_var, &proof_var)
            .unwrap()
            .value()
            .unwrap()
    )
}
