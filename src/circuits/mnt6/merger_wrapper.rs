use std::fs::File;

use algebra::mnt4_753::{Fq, Fr, MNT4_753};
use algebra::mnt6_753::Fr as MNT6Fr;
use algebra_core::CanonicalDeserialize;
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::groth16::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt4_753::PairingGadget;
use r1cs_std::prelude::*;

use crate::circuits::mnt4::MergerCircuit;
use crate::gadgets::input::RecursiveInputGadget;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the wrapper circuit just by editing these types.
type TheProofSystem = Groth16<MNT4_753, MergerCircuit, Fr>;
type TheProofGadget = ProofGadget<MNT4_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT4_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT4_753, Fq, PairingGadget>;

/// This is the merger wrapper circuit. It takes as inputs an initial state commitment, a final state
/// commitment and a verifying key and it produces a proof that there exists a valid SNARK proof that
/// transforms the initial state into the final state.
/// The circuit is basically only a SNARK verifier. Its use is just to change the elliptic curve
/// that the proof exists in, which is sometimes needed for recursive composition of SNARK proofs.
/// This circuit only verifies proofs from the Merger circuit because it has the corresponding
/// verification key hard-coded as a constant.
#[derive(Clone)]
pub struct MergerWrapperCircuit {
    // Private inputs
    proof: Proof<MNT4_753>,

    // Public inputs
    initial_state_commitment: Vec<u8>,
    final_state_commitment: Vec<u8>,
    vk_commitment: Vec<u8>,
}

impl MergerWrapperCircuit {
    pub fn new(
        proof: Proof<MNT4_753>,
        initial_state_commitment: Vec<u8>,
        final_state_commitment: Vec<u8>,
        vk_commitment: Vec<u8>,
    ) -> Self {
        Self {
            proof,
            initial_state_commitment,
            final_state_commitment,
            vk_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT6Fr> for MergerWrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Load the verifying key from file.
        let mut file = File::open("verifying_keys/merger.bin")?;

        let vk_merger = VerifyingKey::deserialize(&mut file).unwrap();

        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let vk_merger_var = TheVkGadget::alloc_constant(cs.ns(|| "alloc vk merger"), &vk_merger)?;

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || { "Alloc private inputs" });

        let proof_var = TheProofGadget::alloc(cs.ns(|| "alloc proof"), || Ok(&self.proof))?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let initial_state_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc initial state commitment"),
            self.initial_state_commitment.as_ref(),
        )?;

        let final_state_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc final state commitment"),
            self.final_state_commitment.as_ref(),
        )?;

        let vk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc vk merger commitment"),
            self.vk_commitment.as_ref(),
        )?;

        // Verify the ZK proof.
        next_cost_analysis!(cs, cost, || { "Verify ZK proof" });

        let mut proof_inputs =
            RecursiveInputGadget::to_field_elements::<Fr>(&initial_state_commitment_var)?;

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &final_state_commitment_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &vk_commitment_var,
        )?);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify groth16 proof"),
            &vk_merger_var,
            proof_inputs.iter(),
            &proof_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
