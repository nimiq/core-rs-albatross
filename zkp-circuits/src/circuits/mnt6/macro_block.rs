use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt6_753::{
    constraints::{G2Var, PairingVar},
    Fq as MNT6Fq, G1Affine, G1Projective, G2Affine, G2Projective, MNT6_753,
};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget, UInt32, UInt8};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::{MacroBlock, PEDERSEN_PARAMETERS};
use rand::Rng;

use super::pk_tree_node::{hash_g2, PkInnerNodeWindow};
use crate::gadgets::{
    mnt6::{DefaultPedersenParametersVar, MacroBlockGadget, StateCommitmentGadget},
    pedersen::PedersenHashGadget,
    recursive_input::RecursiveInputVar,
    serialize::SerializeGadget,
};

/// This is the macro block circuit. It takes as inputs the previous state commitment and final state commitment
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
    proof: Proof<MNT6_753>,
    prev_pk_tree_root: [u8; 95],
    prev_header_hash: [u8; 32],
    final_pk_tree_root: [u8; 95],
    block: MacroBlock,
    l_pk_node_hash: [u8; 95],
    r_pk_node_hash: [u8; 95],
    l_agg_pk_commitment: G2Projective,
    r_agg_pk_commitment: G2Projective,

    // Inputs (public)
    prev_state_commitment: [u8; 95],
    final_state_commitment: [u8; 95],
}

impl MacroBlockCircuit {
    pub fn new(
        vk_pk_tree: VerifyingKey<MNT6_753>,
        proof: Proof<MNT6_753>,
        prev_pk_tree_root: [u8; 95],
        prev_header_hash: [u8; 32],
        final_pk_tree_root: [u8; 95],
        block: MacroBlock,
        l_pk_node_hash: [u8; 95],
        r_pk_node_hash: [u8; 95],
        l_agg_pk_commitment: G2Projective,
        r_agg_pk_commitment: G2Projective,
        prev_state_commitment: [u8; 95],
        final_state_commitment: [u8; 95],
    ) -> Self {
        Self {
            vk_pk_tree,
            proof,
            prev_pk_tree_root,
            prev_header_hash,
            final_pk_tree_root,
            block,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_pk_commitment,
            r_agg_pk_commitment,
            prev_state_commitment,
            final_state_commitment,
        }
    }

    pub fn rand<R: Rng + ?Sized>(vk_child: VerifyingKey<MNT6_753>, rng: &mut R) -> Self {
        let proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let mut prev_pk_tree_root = [0u8; 95];
        rng.fill_bytes(&mut prev_pk_tree_root);

        let mut prev_header_hash = [0u8; 32];
        rng.fill_bytes(&mut prev_header_hash);

        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut final_pk_tree_root = [0u8; 95];
        rng.fill_bytes(&mut final_pk_tree_root);

        let mut prev_state_commitment = [0u8; 95];
        rng.fill_bytes(&mut prev_state_commitment);

        let mut final_state_commitment = [0u8; 95];
        rng.fill_bytes(&mut final_state_commitment);

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let signature = G1Projective::rand(rng);

        let mut signer_bitmap = Vec::with_capacity(Policy::SLOTS as usize);
        for _ in 0..Policy::SLOTS {
            signer_bitmap.push(rng.gen());
        }

        let block = MacroBlock {
            block_number,
            round_number,
            header_hash,
            signature,
            signer_bitmap,
        };

        let mut l_pk_node_hash = [0u8; 95];
        rng.fill_bytes(&mut l_pk_node_hash);

        let mut r_pk_node_hash = [0u8; 95];
        rng.fill_bytes(&mut r_pk_node_hash);

        let l_agg_commitment = G2Projective::rand(rng);

        let r_agg_commitment = G2Projective::rand(rng);

        MacroBlockCircuit::new(
            vk_child,
            proof,
            prev_pk_tree_root,
            prev_header_hash,
            final_pk_tree_root,
            block,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_commitment,
            r_agg_commitment,
            prev_state_commitment,
            final_state_commitment,
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
            PEDERSEN_PARAMETERS.sub_window::<PkInnerNodeWindow>(),
        )?;

        let vk_pk_tree_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(cs.clone(), &self.vk_pk_tree)?;

        // Allocate all the witnesses.
        let proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        let prev_pk_tree_root_bytes =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&self.prev_pk_tree_root[..]))?;

        let prev_header_hash_bytes =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&self.prev_header_hash[..]))?;

        let final_pk_tree_root_bytes =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&self.final_pk_tree_root[..]))?;

        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(&self.block))?;

        let l_pk_node_hash_bytes =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&self.l_pk_node_hash[..]))?;

        let r_pk_node_hash_bytes =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&self.r_pk_node_hash[..]))?;

        let l_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.l_agg_pk_commitment))?;

        let r_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.r_agg_pk_commitment))?;

        let prev_block_number_var = UInt32::new_witness(cs.clone(), || {
            Ok(self.block.block_number - Policy::blocks_per_epoch())
        })?;

        // Allocate all the inputs.
        let prev_state_commitment_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.prev_state_commitment[..])?;

        let final_state_commitment_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.final_state_commitment[..])?;

        // Check that the previous block and the final block are exactly one epoch length apart.
        let calculated_block_number =
            UInt32::addmany(&[prev_block_number_var.clone(), epoch_length_var])?;

        calculated_block_number.enforce_equal(&block_var.block_number)?;

        // Verifying equality for previous state commitment. It just checks that the previous block
        // number, header hash and public key tree root given as witnesses are correct by committing
        // to them and comparing the result with the previous state commitment given as an input.
        let reference_commitment = StateCommitmentGadget::evaluate(
            cs.clone(),
            &prev_block_number_var,
            &prev_header_hash_bytes,
            &prev_pk_tree_root_bytes,
            &pedersen_generators_var,
        )?;

        prev_state_commitment_bytes.enforce_equal(&reference_commitment)?;

        // Verifying equality for final state commitment. It just checks that the final block number,
        // header hash and public key tree root given as a witnesses are correct by committing
        // to them and comparing the result with the final state commitment given as an input.
        let reference_commitment = StateCommitmentGadget::evaluate(
            cs.clone(),
            &block_var.block_number,
            &block_var.header_hash,
            &final_pk_tree_root_bytes,
            &pedersen_generators_var,
        )?;

        final_state_commitment_bytes.enforce_equal(&reference_commitment)?;

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
        let pedersen_hash = PedersenHashGadget::<_, _, PkInnerNodeWindow>::evaluate(
            &pk_node_hash_bytes,
            &pedersen_generators_var,
        )?;
        let pk_node_hash_bytes = pedersen_hash.serialize_compressed(cs.clone())?;

        pk_node_hash_bytes.enforce_equal(&prev_pk_tree_root_bytes)?;

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
        proof_inputs.push(&block_var.signer_bitmap)?;

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_pk_tree_var,
            &proof_inputs.into(),
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Calculating the aggregate public key.
        let agg_pk_var = l_agg_pk_commitment_var + r_agg_pk_commitment_var;

        // Verifying that the block is valid.
        block_var
            .verify(cs, &final_pk_tree_root_bytes, &agg_pk_var)?
            .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
