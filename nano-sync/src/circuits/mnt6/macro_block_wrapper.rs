

use ark_crypto_primitives::snark::BooleanInputVar;
use ark_crypto_primitives::SNARKGadget;
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::{Proof, VerifyingKey};
use ark_mnt4_753::constraints::{FqVar, PairingVar};
use ark_mnt4_753::{Fq, MNT4_753};
use ark_mnt6_753::Fr as MNT6Fr;
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};


use crate::utils::{pack_inputs, unpack_inputs};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

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
    // Our inputs are always vectors of booleans (semantically), so that they are consistent across
    // the different elliptic curves that we use. However, for compactness, we represent them as
    // field elements. Both of the curves that we use have a modulus of 753 bits and a capacity
    // of 752 bits. So, the first 752 bits (in little-endian) of each field element is data, and the
    // last bit is always set to zero.
    initial_state_commitment: Vec<Fq>,
    final_state_commitment: Vec<Fq>,
}

impl MacroBlockWrapperCircuit {
    pub fn new(
        vk_macro_block: VerifyingKey<MNT4_753>,
        proof: Proof<MNT4_753>,
        initial_state_commitment: Vec<Fq>,
        final_state_commitment: Vec<Fq>,
    ) -> Self {
        Self {
            vk_macro_block,
            proof,
            initial_state_commitment,
            final_state_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT6Fr> for MacroBlockWrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fr>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let vk_macro_block_var = VerifyingKeyVar::<MNT4_753, PairingVar>::new_constant(
            cs.clone(),
            &self.vk_macro_block,
        )?;

        // Allocate all the witnesses.
        next_cost_analysis!(cs, cost, || { "Alloc witnesses" });

        let proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        // Allocate all the inputs.
        next_cost_analysis!(cs, cost, || { "Alloc inputs" });

        let initial_state_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.initial_state_commitment[..]))?;

        let final_state_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.final_state_commitment[..]))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        next_cost_analysis!(cs, cost, || { "Unpack inputs" });

        let initial_state_commitment_bits =
            unpack_inputs(initial_state_commitment_var)?[..760].to_vec();

        let final_state_commitment_bits =
            unpack_inputs(final_state_commitment_var)?[..760].to_vec();

        // Verify the ZK proof.
        next_cost_analysis!(cs, cost, || { "Verify ZK proof" });

        let mut proof_inputs = pack_inputs(initial_state_commitment_bits);

        proof_inputs.append(&mut pack_inputs(final_state_commitment_bits));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_macro_block_var,
            &input_var,
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
