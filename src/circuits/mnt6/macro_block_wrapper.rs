use algebra::mnt4_753::{Fq, Fr, MNT4_753};
use algebra::mnt6_753::Fr as MNT6Fr;
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::groth16::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt4_753::PairingGadget;
use r1cs_std::prelude::*;

use crate::circuits::mnt4::MacroBlockCircuit;
use crate::gadgets::input::RecursiveInputGadget;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the wrapper circuit just by editing these types.
type TheProofSystem = Groth16<MNT4_753, MacroBlockCircuit, Fr>;
type TheProofGadget = ProofGadget<MNT4_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT4_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT4_753, Fq, PairingGadget>;

/// This is the macro block wrapper circuit. It takes as inputs an initial state commitment and a
/// final state commitment and it produces a proof that there exists a valid SNARK proof that transforms
/// the initial state into the final state.
/// The circuit is basically only a SNARK verifier. Its use is just to change the elliptic curve
/// that the proof exists in, which is sometimes needed for recursive composition of SNARK proofs.
/// This circuit only verifies proofs from the Macro Block circuit because it has the corresponding
/// verification key hard-coded as a constant.
#[derive(Clone)]
pub struct MacroBlockWrapperCircuit {
    // Private inputs
    proof: Proof<MNT4_753>,
    vk_macro_block: VerifyingKey<MNT4_753>,

    // Public inputs
    initial_state_commitment: Vec<u8>,
    final_state_commitment: Vec<u8>,
}

impl MacroBlockWrapperCircuit {
    pub fn new(
        proof: Proof<MNT4_753>,
        vk_macro_block: VerifyingKey<MNT4_753>,
        initial_state_commitment: Vec<u8>,
        final_state_commitment: Vec<u8>,
    ) -> Self {
        Self {
            proof,
            vk_macro_block,
            initial_state_commitment,
            final_state_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT6Fr> for MacroBlockWrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        // TODO: This needs to be changed to a constant!
        let vk_macro_block_var =
            TheVkGadget::alloc(
                cs.ns(|| "alloc vk macro block"),
                || Ok(&self.vk_macro_block),
            )?;

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

        // Verify the ZK proof.
        next_cost_analysis!(cs, cost, || { "Verify ZK proof" });
        let mut proof_inputs =
            RecursiveInputGadget::to_field_elements::<Fr>(&initial_state_commitment_var)?;
        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &final_state_commitment_var,
        )?);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify groth16 proof"),
            &vk_macro_block_var,
            proof_inputs.iter(),
            &proof_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
