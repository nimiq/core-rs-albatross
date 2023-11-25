use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar},
    Proof,
};
use ark_mnt4_753::{constraints::PairingVar, Fq as MNT4Fq, G1Affine, G2Affine, MNT4_753};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use nimiq_zkp_primitives::pedersen::pedersen_parameters_mnt4;
use rand::Rng;

use crate::{
    circuits::{
        num_inputs,
        vk_commitments::{CircuitId, VerifyingKeyHelper, VerifyingKeys},
        CircuitInput,
    },
    gadgets::{
        mnt4::DefaultPedersenParametersVar, recursive_input::RecursiveInputVar,
        vk_commitment::VkCommitmentWindow,
    },
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
    // Witnesses (private)
    keys: VerifyingKeys,
    proof: Proof<MNT4_753>,

    // Inputs (public)
    prev_header_hash: [u8; 32],
    final_header_hash: [u8; 32],
    vks_commitment: [u8; 95 * 2],
}

impl CircuitInput for MacroBlockWrapperCircuit {
    const NUM_INPUTS: usize = num_inputs::<MNT4_753>(&[32, 32, 95 * 2]);
}

impl MacroBlockWrapperCircuit {
    pub fn new(
        keys: VerifyingKeys,
        proof: Proof<MNT4_753>,
        prev_header_hash: [u8; 32],
        final_header_hash: [u8; 32],
    ) -> Self {
        Self {
            vks_commitment: keys.commitment(),
            keys,
            proof,
            prev_header_hash,
            final_header_hash,
        }
    }

    pub fn rand<R: Rng + ?Sized>(rng: &mut R) -> Self {
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

        let keys = VerifyingKeys::rand(rng);

        // Create parameters for our circuit
        MacroBlockWrapperCircuit::new(keys, proof, prev_header_hash, prev_header_hash)
    }
}

impl ConstraintSynthesizer<MNT4Fq> for MacroBlockWrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fq>) -> Result<(), SynthesisError> {
        // Allocate constants.
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt4().sub_window::<VkCommitmentWindow>(),
        )?;

        // Allocate all the witnesses.
        let proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        // Allocate all the inputs.
        let prev_header_hash_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.prev_header_hash)?;
        let final_header_hash_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.final_header_hash)?;
        let vks_commitment_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.vks_commitment)?;

        // Allocate the vk gadget.
        let vk_helper = VerifyingKeyHelper::new_and_verify::<PairingVar>(
            cs.clone(),
            self.keys.clone(),
            &vks_commitment_var,
            &pedersen_generators,
        )?;

        // Get merger vk.
        let macro_block_vk = vk_helper.get_and_verify_vk::<_, VkCommitmentWindow>(
            cs,
            CircuitId::MacroBlock,
            &pedersen_generators,
        )?;

        // Verify the ZK proof.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&prev_header_hash_var)?;
        proof_inputs.push(&final_header_hash_var)?;
        proof_inputs.push(&vks_commitment_var)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &macro_block_vk,
            &proof_inputs.into(),
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
