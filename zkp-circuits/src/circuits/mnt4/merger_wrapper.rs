use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar},
    Proof, VerifyingKey,
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
    circuits::{num_inputs, CircuitInput},
    gadgets::{
        mnt4::DefaultPedersenParametersVar,
        recursive_input::RecursiveInputVar,
        vk_commitment::{VkCommitmentGadget, VkCommitmentWindow},
    },
};

/// This is the merger wrapper circuit. It takes as inputs the genesis header hash, a final state
/// commitment and a verifying key commitment and it produces a proof that there exists a valid SNARK
/// proof that transforms the genesis state into the final state.
/// The circuit is basically only a SNARK verifier. Its use is just to change the elliptic curve
/// that the proof exists in, which is sometimes needed for recursive composition of SNARK proofs.
/// This circuit only verifies proofs from the Merger circuit because it has the corresponding
/// verification key hard-coded as a constant.
#[derive(Clone)]
pub struct MergerWrapperCircuit {
    // Witnesses (private)
    merger_vk: VerifyingKey<MNT4_753>,
    proof: Proof<MNT4_753>,

    // Inputs (public)
    genesis_header_hash: [u8; 32],
    final_header_hash: [u8; 32],
    vks_commitment: [u8; 95],
    // The merger vk commitment is an implicit input and derived from the merger_vk.
    // merger_vk_commitment: [u8; 95],
}

impl CircuitInput for MergerWrapperCircuit {
    const NUM_INPUTS: usize = num_inputs::<MNT4_753>(&[32, 32, 95, 95]);
}

impl MergerWrapperCircuit {
    pub fn new(
        merger_vk: VerifyingKey<MNT4_753>,
        proof: Proof<MNT4_753>,
        genesis_header_hash: [u8; 32],
        final_header_hash: [u8; 32],
        vks_commitment: [u8; 95],
    ) -> Self {
        Self {
            merger_vk,
            proof,
            genesis_header_hash,
            final_header_hash,
            vks_commitment,
        }
    }

    pub fn rand<R: Rng + ?Sized>(vk_child: VerifyingKey<MNT4_753>, rng: &mut R) -> Self {
        // Create dummy inputs.
        let proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let mut genesis_header_hash = [0u8; 32];
        rng.fill_bytes(&mut genesis_header_hash);

        let mut final_header_hash = [0u8; 32];
        rng.fill_bytes(&mut final_header_hash);

        let mut vks_commitment = [0u8; 95];
        rng.fill_bytes(&mut vks_commitment);

        // Create parameters for our circuit
        MergerWrapperCircuit::new(
            vk_child,
            proof,
            genesis_header_hash,
            final_header_hash,
            vks_commitment,
        )
    }
}

impl ConstraintSynthesizer<MNT4Fq> for MergerWrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fq>) -> Result<(), SynthesisError> {
        // Allocate all the witnesses.
        let proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        // Allocate all the inputs.
        let genesis_header_hash_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.genesis_header_hash)?;
        let final_header_hash_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.final_header_hash)?;
        let vks_commitment_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.vks_commitment)?;

        // Allocate the vk gadget.
        // This allocates both a witness and an input.
        let merger_vk_gadget =
            VkCommitmentGadget::new(cs.clone(), &self.merger_vk, vks_commitment_var.clone())?;

        // Verify the merger vk.
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt4().sub_window::<VkCommitmentWindow>(),
        )?;
        merger_vk_gadget.verify(cs.clone(), &pedersen_generators)?;

        // Verify the ZK proof.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&genesis_header_hash_var)?;
        proof_inputs.push(&final_header_hash_var)?;
        proof_inputs.push(&vks_commitment_var)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &merger_vk_gadget.vk,
            &proof_inputs.into(),
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
