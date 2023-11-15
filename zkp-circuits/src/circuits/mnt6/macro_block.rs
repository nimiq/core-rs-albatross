use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt6_753::{
    constraints::{G2Var, PairingVar},
    Fq as MNT6Fq, G1Affine, G2Affine, G2Projective, MNT6_753,
};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget, UInt32, UInt8};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use nimiq_block::MacroBlock;
use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::pedersen_parameters_mnt6;
use rand::Rng;

use super::pk_tree_node::{hash_g2, PkInnerNodeWindow};
use crate::{
    blake2s::evaluate_blake2s,
    gadgets::{
        mnt6::{DefaultPedersenParametersVar, MacroBlockGadget},
        recursive_input::RecursiveInputVar,
    },
};

/// This is the macro block circuit. It takes as inputs the previous header hash and final header hash
/// and it produces a proof that there exists a valid macro block that transforms the previous state
/// into the final state.
/// Since the state is composed only of the block number and the public keys of the current validator
/// list, updating the state is just incrementing the block number and substituting the previous
/// public keys with the public keys of the new validator list.
#[derive(Clone)]
pub struct MacroBlockCircuit {
    // Verifying key for the PKTree circuit. Not an input to the SNARK circuit.
    vk_pk_tree: VerifyingKey<MNT6_753>,

    // Witnesses (private)
    prev_pk_tree_proof: Proof<MNT6_753>,
    l_pk_node_hash: [u8; 32],
    r_pk_node_hash: [u8; 32],
    l_agg_pk_commitment: G2Projective,
    r_agg_pk_commitment: G2Projective,
    prev_block: MacroBlock,
    final_block: MacroBlock,

    // Inputs (public)
    pub prev_header_hash: [u8; 32],
    pub final_header_hash: [u8; 32],
}

impl MacroBlockCircuit {
    pub fn new(
        vk_pk_tree: VerifyingKey<MNT6_753>,
        prev_pk_tree_proof: Proof<MNT6_753>,
        l_pk_node_hash: [u8; 32],
        r_pk_node_hash: [u8; 32],
        l_agg_pk_commitment: G2Projective,
        r_agg_pk_commitment: G2Projective,
        prev_block: MacroBlock,
        final_block: MacroBlock,
    ) -> Self {
        let prev_header_hash = prev_block.hash_blake2s().0;
        let final_header_hash = final_block.hash_blake2s().0;

        Self {
            vk_pk_tree,
            prev_pk_tree_proof,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_pk_commitment,
            r_agg_pk_commitment,
            prev_block,
            final_block,
            prev_header_hash,
            final_header_hash,
        }
    }

    pub fn rand<R: Rng + ?Sized>(vk_child: VerifyingKey<MNT6_753>, rng: &mut R) -> Self {
        let proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        // PITODO: randomize?
        let mut prev_block = MacroBlock::non_empty_default();
        prev_block.header.block_number = u32::rand(rng);
        let mut final_block = MacroBlock::non_empty_default();
        final_block.header.block_number = u32::rand(rng);

        let mut l_pk_node_hash = [0u8; 32];
        rng.fill_bytes(&mut l_pk_node_hash);

        let mut r_pk_node_hash = [0u8; 32];
        rng.fill_bytes(&mut r_pk_node_hash);

        let l_agg_commitment = G2Projective::rand(rng);

        let r_agg_commitment = G2Projective::rand(rng);

        MacroBlockCircuit::new(
            vk_child,
            proof,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_commitment,
            r_agg_commitment,
            prev_block,
            final_block,
        )
    }
}

impl ConstraintSynthesizer<MNT6Fq> for MacroBlockCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let epoch_length_var =
            UInt32::<MNT6Fq>::new_constant(cs.clone(), Policy::blocks_per_epoch())?;

        let pedersen_generators_var = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<PkInnerNodeWindow>(),
        )?;

        let vk_pk_tree_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(cs.clone(), &self.vk_pk_tree)?;

        // Allocate all the witnesses.
        let proof_var = ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || {
            Ok(&self.prev_pk_tree_proof)
        })?;

        let l_pk_node_hash_bytes =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &self.l_pk_node_hash)?;
        let r_pk_node_hash_bytes =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &self.r_pk_node_hash)?;

        let l_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.l_agg_pk_commitment))?;
        let r_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.r_agg_pk_commitment))?;

        let mut prev_block_var =
            MacroBlockGadget::new_witness(cs.clone(), || Ok(&self.prev_block))?;
        let mut final_block_var =
            MacroBlockGadget::new_witness(cs.clone(), || Ok(&self.final_block))?;

        // Inputs
        let prev_header_hash_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.prev_header_hash)?;
        let final_header_hash_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.final_header_hash)?;

        // Check that the previous block and the final block are exactly one epoch length apart.
        let calculated_block_number =
            UInt32::addmany(&[prev_block_var.block_number.clone(), epoch_length_var])?;

        calculated_block_number.enforce_equal(&final_block_var.block_number)?;

        // TODO: If we move to Blake2s, we might want to check the parent hash.
        // Enforce equality on header hashes.
        prev_block_var
            .hash(cs.clone())?
            .enforce_equal(&prev_header_hash_bytes)?;
        final_block_var
            .hash(cs.clone())?
            .enforce_equal(&final_header_hash_bytes)?;

        // Calculating the commitments to each of the aggregate public keys chunks. These will be
        // given as inputs to the PKTree SNARK circuit.
        let l_agg_pk_commitment_bytes =
            hash_g2(&cs, &l_agg_pk_commitment_var, &pedersen_generators_var)?;
        let r_agg_pk_commitment_bytes =
            hash_g2(&cs, &r_agg_pk_commitment_var, &pedersen_generators_var)?;

        // Calculate and verify the root hash.
        let mut pk_node_hash_bytes = vec![];
        pk_node_hash_bytes.extend_from_slice(&l_pk_node_hash_bytes);
        pk_node_hash_bytes.extend_from_slice(&r_pk_node_hash_bytes);
        let pk_node_hash_bytes = evaluate_blake2s(&pk_node_hash_bytes)?;

        pk_node_hash_bytes.enforce_equal(&prev_block_var.pk_tree_root)?;

        // Verifying the SNARK proof. This is a proof that the aggregate public key is indeed
        // correct. It simply takes the public keys and the signers bitmap and recalculates the
        // aggregate public key and then compares it to the aggregate public key given as public
        // input.
        // Internally, this SNARK circuit is very complex. See the PKTreeLeaf circuit for more details.
        // Note that in this particular case, we don't pass the aggregated public key to the SNARK.
        // Instead we pass two chunks of the aggregated public key to it. This is just because the
        // PKTreeNode circuit in the MNT6 curve takes two chunks as inputs.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&l_pk_node_hash_bytes)?;
        proof_inputs.push(&r_pk_node_hash_bytes)?;
        proof_inputs.push(&l_agg_pk_commitment_bytes)?;
        proof_inputs.push(&r_agg_pk_commitment_bytes)?;
        proof_inputs.push(&final_block_var.signer_bitmap)?;

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_pk_tree_var,
            &proof_inputs.into(),
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Calculating the aggregate public key.
        let agg_pk_var = l_agg_pk_commitment_var + r_agg_pk_commitment_var;

        // Verifying that the block is valid.
        final_block_var
            .verify_signature(cs, &agg_pk_var)?
            .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
