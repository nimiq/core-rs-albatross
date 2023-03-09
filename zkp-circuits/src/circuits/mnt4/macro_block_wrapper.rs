use ark_crypto_primitives::{
    snark::BooleanInputVar,
    snark::{FromFieldElementsGadget, SNARKGadget},
};
use ark_ff::ToConstraintField;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt4_753::{
    constraints::{FqVar, PairingVar},
    Fq as MNT4Fq, MNT4_753,
};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

/// This is the macro block wrapper circuit. It takes as inputs an initial state commitment and a
/// final state commitment and it produces a proof that there exists a valid SNARK proof that transforms
/// the initial state into the final state.
/// The circuit is basically only a SNARK verifier. Its use is just to change the elliptic curve
/// that the proof exists in, which is sometimes needed for recursive composition of SNARK proofs.
/// This circuit only verifies proofs from the Macro Block circuit because it has the corresponding
/// verification key hard-coded as a constant.
#[derive(Clone)]
pub struct MacroBlockWrapperCircuit {
    // Verifying key for the macro block circuit. Not an input to the SNARK circuit.
    vk_macro_block: VerifyingKey<MNT4_753>,

    // Witnesses (private)
    proof: Proof<MNT4_753>,

    // Inputs (public)
    initial_state_commitment: [u8; 95],
    final_state_commitment: [u8; 95],
}

impl MacroBlockWrapperCircuit {
    pub fn new(
        vk_macro_block: VerifyingKey<MNT4_753>,
        proof: Proof<MNT4_753>,
        initial_state_commitment: [u8; 95],
        final_state_commitment: [u8; 95],
    ) -> Self {
        Self {
            vk_macro_block,
            proof,
            initial_state_commitment,
            final_state_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fq> for MacroBlockWrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fq>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let vk_macro_block_var = VerifyingKeyVar::<MNT4_753, PairingVar>::new_constant(
            cs.clone(),
            &self.vk_macro_block,
        )?;

        // Allocate all the witnesses.
        let proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        // Allocate all the inputs.
        // Since we're only passing it through, we directly allocate them as FqVars.
        let initial_state_commitment_var = Vec::<FqVar>::new_input(cs.clone(), || {
            self.initial_state_commitment
                .to_field_elements()
                .ok_or(SynthesisError::AssignmentMissing)
        })?;

        let mut final_state_commitment_var = Vec::<FqVar>::new_input(cs, || {
            self.final_state_commitment
                .to_field_elements()
                .ok_or(SynthesisError::AssignmentMissing)
        })?;

        // Verify the ZK proof.
        let mut proof_inputs = initial_state_commitment_var;

        proof_inputs.append(&mut final_state_commitment_var);

        let input_var = BooleanInputVar::from_field_elements(&proof_inputs)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_macro_block_var,
            &input_var,
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
