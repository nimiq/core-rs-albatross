use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt6_753::{constraints::PairingVar, Fq as MNT6Fq, G1Affine, G2Affine, MNT6_753};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use nimiq_zkp_primitives::pedersen_parameters;
use rand::Rng;

use crate::gadgets::{
    mnt6::DefaultPedersenParametersVar,
    recursive_input::RecursiveInputVar,
    vk_commitment::{VKCommitmentGadget, VkCommitmentWindow},
};

/// This is the merger circuit. It takes as inputs the genesis header hash, a final header hash
/// and a verifying key and it produces a proof that there exist two valid SNARK proofs that transform
/// the genesis state into the final state passing through some intermediate state.
/// The circuit is composed of two SNARK verifiers in a row. It's used to verify a Merger Wrapper proof
/// and a Macro Block Wrapper proof, effectively merging both into a single proof. Evidently, this is
/// needed for recursive composition of SNARK proofs.
/// This circuit has the verification key for the Macro Block Wrapper hard-coded as a constant, but the
/// verification key for the Merger Wrapper is given as a witness (which is then checked against the
/// verification key commitment provided as an input).
/// To guarantee that the prover inputs the correct Merger Wrapper verification key, the verifier also
/// supplies a commitment to the desired verification key as a public input. This circuit then enforces
/// the equality of the commitment and of the verification key.
/// Additionally, the prover can set (as a private input) a boolean flag determining if this circuit
/// is evaluating the first epoch or not. If the flag is set to true, the circuit will enforce that
/// the genesis state and the intermediate state are equal but it will not enforce the verification of
/// the Merger Wrapper proof. If the flag is set to false, the circuit will enforce the verification
/// of the Merger Wrapper proof, but it will not enforce the equality of the genesis and intermediate
/// states.
/// The rationale is that, for the first epoch, the merger circuit will not have any previous Merger
/// Wrapper proof to verify since there are no previous state changes. But in that case, the genesis
/// and intermediate states must be equal by definition.
#[derive(Clone)]
pub struct MergerCircuit {
    // Verifying key for the macro block wrapper circuit. Not an input to the SNARK circuit.
    vk_macro_block_wrapper: VerifyingKey<MNT6_753>,

    // Witnesses (private)
    proof_merger_wrapper: Proof<MNT6_753>,
    proof_macro_block_wrapper: Proof<MNT6_753>,
    vk_merger_wrapper: VerifyingKey<MNT6_753>,
    intermediate_header_hash: [u8; 32],
    genesis_flag: bool,

    // Inputs (public)
    genesis_header_hash: [u8; 32],
    final_header_hash: [u8; 32],
}

impl MergerCircuit {
    pub fn new(
        vk_macro_block_wrapper: VerifyingKey<MNT6_753>,
        proof_merger_wrapper: Proof<MNT6_753>,
        proof_macro_block_wrapper: Proof<MNT6_753>,
        vk_merger_wrapper: VerifyingKey<MNT6_753>,
        intermediate_header_hash: [u8; 32],
        genesis_flag: bool,
        genesis_header_hash: [u8; 32],
        final_header_hash: [u8; 32],
    ) -> Self {
        Self {
            vk_macro_block_wrapper,
            proof_merger_wrapper,
            proof_macro_block_wrapper,
            vk_merger_wrapper,
            intermediate_header_hash,
            genesis_flag,
            genesis_header_hash,
            final_header_hash,
        }
    }

    pub fn rand<R: Rng + ?Sized>(vk_child: VerifyingKey<MNT6_753>, rng: &mut R) -> Self {
        // Create dummy inputs.
        let proof_merger_wrapper = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let proof_macro_block_wrapper = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let vk_merger_wrapper = VerifyingKey {
            alpha_g1: G1Affine::rand(rng),
            beta_g2: G2Affine::rand(rng),
            gamma_g2: G2Affine::rand(rng),
            delta_g2: G2Affine::rand(rng),
            gamma_abc_g1: vec![G1Affine::rand(rng); 5],
        };

        let mut intermediate_header_hash = [0u8; 32];
        rng.fill_bytes(&mut intermediate_header_hash);

        let genesis_flag = bool::rand(rng);

        let mut genesis_header_hash = [0u8; 32];
        rng.fill_bytes(&mut genesis_header_hash);

        let mut final_header_hash = [0u8; 32];
        rng.fill_bytes(&mut final_header_hash);

        // Create parameters for our circuit
        MergerCircuit::new(
            vk_child,
            proof_merger_wrapper,
            proof_macro_block_wrapper,
            vk_merger_wrapper,
            intermediate_header_hash,
            genesis_flag,
            genesis_header_hash,
            final_header_hash,
        )
    }
}

impl ConstraintSynthesizer<MNT6Fq> for MergerCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<(), SynthesisError> {
        let vk_macro_block_wrapper_var = VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(
            cs.clone(),
            &self.vk_macro_block_wrapper,
        )?;

        // Allocate all the witnesses.
        let proof_merger_wrapper_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || {
                Ok(&self.proof_merger_wrapper)
            })?;

        let proof_macro_block_wrapper_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || {
                Ok(&self.proof_macro_block_wrapper)
            })?;

        // let vk_merger_wrapper_var =
        //     VerifyingKeyVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || {
        //         Ok(&self.vk_merger_wrapper)
        //     })?;

        let intermediate_header_hash_bytes =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &self.intermediate_header_hash)?;

        let genesis_flag_var = Boolean::new_witness(cs.clone(), || Ok(&self.genesis_flag))?;

        // Allocate all the inputs.
        // Since we're only passing them through, allocate as Vec<FqVar>
        let genesis_header_hash_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.genesis_header_hash)?;

        let final_header_hash_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.final_header_hash)?;

        let vk_commitment =
            VKCommitmentGadget::new_native(cs.clone(), Some(self.vk_merger_wrapper.clone()), 4)?;
        // let vk_commitment_bytes = UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.vk_commitment)?;

        // Verify equality for vk commitment. It just checks that the private input is correct by
        // committing to it and then comparing the result with the vk commitment given as a public input.
        // let reference_commitment = VKCommitmentGadget::evaluate(cs, &vk_merger_wrapper_var)?;

        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters().sub_window::<VkCommitmentWindow>(),
        )?;
        // vk_commitment_bytes.enforce_equal(&reference_commitment)?;
        vk_commitment
            .verify(cs.clone(), &pedersen_generators)?
            .enforce_equal(&Boolean::constant(true))?;

        // Verify equality of genesis and intermediate header hashes. If the genesis flag is set to
        // true, it enforces the equality. If it is set to false, it doesn't. This is necessary for
        // the genesis block, for the first merger circuit.
        genesis_header_hash_bytes
            .conditional_enforce_equal(&intermediate_header_hash_bytes, &genesis_flag_var)?;

        // Verify the ZK proof for the Merger Wrapper circuit. If the genesis flag is set to false,
        // it enforces the verification. If it is set to true, it doesn't. This is necessary for
        // the first epoch, for the first merger circuit.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&genesis_header_hash_bytes)?;
        proof_inputs.push(&intermediate_header_hash_bytes)?;
        proof_inputs.push(&vk_commitment.vk_commitment)?;

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            vk_commitment
                .vk
                .native()
                .ok_or(SynthesisError::MalformedVerifyingKey)?,
            &proof_inputs.into(),
            &proof_merger_wrapper_var,
        )?
        .enforce_equal(&genesis_flag_var.not())?;

        // Verify the ZK proof for the Macro Block Wrapper circuit.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&intermediate_header_hash_bytes)?;
        proof_inputs.push(&final_header_hash_bytes)?;

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_macro_block_wrapper_var,
            &proof_inputs.into(),
            &proof_macro_block_wrapper_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
