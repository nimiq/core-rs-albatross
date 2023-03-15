use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt4_753::{constraints::PairingVar, Fq as MNT4Fq, G1Affine, G2Affine, MNT4_753};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use rand::Rng;

use crate::gadgets::recursive_input::RecursiveInputVar;

/// This is the merger wrapper circuit. It takes as inputs an initial state commitment, a final state
/// commitment and a verifying key commitment and it produces a proof that there exists a valid SNARK
/// proof that transforms the initial state into the final state.
/// The circuit is basically only a SNARK verifier. Its use is just to change the elliptic curve
/// that the proof exists in, which is sometimes needed for recursive composition of SNARK proofs.
/// This circuit only verifies proofs from the Merger circuit because it has the corresponding
/// verification key hard-coded as a constant.
#[derive(Clone)]
pub struct MergerWrapperCircuit {
    // Verifying key for the merger circuit. Not an input to the SNARK circuit.
    vk_merger: VerifyingKey<MNT4_753>,

    // Witnesses (private)
    proof: Proof<MNT4_753>,

    // Inputs (public)
    initial_state_commitment: [u8; 95],
    final_state_commitment: [u8; 95],
    vk_commitment: [u8; 95],
}

impl MergerWrapperCircuit {
    pub fn new(
        vk_merger: VerifyingKey<MNT4_753>,
        proof: Proof<MNT4_753>,
        initial_state_commitment: [u8; 95],
        final_state_commitment: [u8; 95],
        vk_commitment: [u8; 95],
    ) -> Self {
        Self {
            vk_merger,
            proof,
            initial_state_commitment,
            final_state_commitment,
            vk_commitment,
        }
    }

    pub fn rand<R: Rng + ?Sized>(vk_child: VerifyingKey<MNT4_753>, rng: &mut R) -> Self {
        // Create dummy inputs.
        let proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let mut initial_state_commitment = [0u8; 95];
        rng.fill_bytes(&mut initial_state_commitment);

        let mut final_state_commitment = [0u8; 95];
        rng.fill_bytes(&mut final_state_commitment);

        let mut vk_commitment = [0u8; 95];
        rng.fill_bytes(&mut vk_commitment);

        // Create parameters for our circuit
        MergerWrapperCircuit::new(
            vk_child,
            proof,
            initial_state_commitment,
            final_state_commitment,
            vk_commitment,
        )
    }
}

impl ConstraintSynthesizer<MNT4Fq> for MergerWrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fq>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let vk_merger_var =
            VerifyingKeyVar::<MNT4_753, PairingVar>::new_constant(cs.clone(), &self.vk_merger)?;

        // Allocate all the witnesses.
        let proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        // Allocate all the inputs.
        let initial_state_commitment_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.initial_state_commitment)?;

        let final_state_commitment_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.final_state_commitment)?;

        let vk_commitment_var = UInt8::<MNT4Fq>::new_input_vec(cs, &self.vk_commitment)?;

        // Verify the ZK proof.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&initial_state_commitment_var)?;
        proof_inputs.push(&final_state_commitment_var)?;
        proof_inputs.push(&vk_commitment_var)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_merger_var,
            &proof_inputs.into(),
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
