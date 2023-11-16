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

use crate::{
    circuits::{num_inputs, CircuitInput},
    gadgets::recursive_input::RecursiveInputVar,
};

/// This is the macro block wrapper circuit. It takes as inputs the previous header hash and a
/// final header hash and it produces a proof that there exists a valid SNARK proof that transforms
/// the previous state into the final state.
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
    prev_header_hash: [u8; 32],
    final_header_hash: [u8; 32],
}

impl CircuitInput for MacroBlockWrapperCircuit {
    const NUM_INPUTS: usize = num_inputs::<MNT4_753>(&[32, 32]);
}

impl MacroBlockWrapperCircuit {
    pub fn new(
        vk_macro_block: VerifyingKey<MNT4_753>,
        proof: Proof<MNT4_753>,
        prev_header_hash: [u8; 32],
        final_header_hash: [u8; 32],
    ) -> Self {
        Self {
            vk_macro_block,
            proof,
            prev_header_hash,
            final_header_hash,
        }
    }

    pub fn rand<R: Rng + ?Sized>(vk_child: VerifyingKey<MNT4_753>, rng: &mut R) -> Self {
        // Create dummy inputs.
        let proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let mut prev_header_hash = [0u8; 32];
        rng.fill_bytes(&mut prev_header_hash);

        let mut prev_header_hash = [0u8; 32];
        rng.fill_bytes(&mut prev_header_hash);

        // Create parameters for our circuit
        MacroBlockWrapperCircuit::new(vk_child, proof, prev_header_hash, prev_header_hash)
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
        let prev_header_hash_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.prev_header_hash)?;

        let final_header_hash_var = UInt8::<MNT4Fq>::new_input_vec(cs, &self.final_header_hash)?;

        // Verify the ZK proof.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&prev_header_hash_var)?;
        proof_inputs.push(&final_header_hash_var)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_macro_block_var,
            &proof_inputs.into(),
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
