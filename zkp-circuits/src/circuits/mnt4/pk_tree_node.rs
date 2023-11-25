use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof,
};
use ark_mnt4_753::{constraints::PairingVar, Fq as MNT4Fq, G1Affine, G2Affine, MNT4_753};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use nimiq_primitives::policy::Policy;
use rand::Rng;

use crate::{
    circuits::{
        mnt6::{PKTreeLeafCircuit, PKTreeNodeCircuit as OtherPKTreeNodeCircuit},
        vk_commitments::{CircuitId, PairingRelatedKeys, VerifyingKeys},
    },
    gadgets::{bits::BitVec, compressed_vk::CompressedInput, recursive_input::RecursiveInputVar},
};

/// This is the node subcircuit of the PKTreeCircuit. See PKTreeLeafCircuit for more details.
/// Its purpose it two-fold:
///     1) Split the signer bitmap chunk it receives as an input into two halves.
///     2) Verify the proofs from its two child nodes in the PKTree.
/// It is different from the other node subcircuit on the MNT4 curve in that it doesn't recalculate
/// the aggregate public key commitments, it just passes them on to the next level.
#[derive(Clone)]
pub struct PKTreeNodeCircuit {
    // Constants for the circuit. Careful, changing these values result in a different circuit, so
    // whenever you change these values you need to generate new proving and verifying keys.
    tree_level: usize,

    // Witnesses (private)
    l_proof: Proof<MNT4_753>,
    r_proof: Proof<MNT4_753>,

    // Inputs (public)
    l_pk_node_hash: [u8; 32],
    r_pk_node_hash: [u8; 32],
    l_agg_pk_commitment: [u8; 95],
    r_agg_pk_commitment: [u8; 95],
    signer_bitmap_chunk: Vec<bool>,
    vks_commitment: [u8; 95 * 2],
    keys: VerifyingKeys,
}

impl PKTreeNodeCircuit {
    pub fn num_inputs(tree_level: usize) -> usize {
        let num_bits = Policy::SLOTS as usize / 2_usize.pow(tree_level as u32);
        let num_sub_inputs = if tree_level == 4 {
            PKTreeLeafCircuit::num_inputs(tree_level + 1)
        } else {
            OtherPKTreeNodeCircuit::num_inputs(tree_level + 1)
        };
        let vk_size = /* num G1 */ (2 + num_sub_inputs) + /* G2 size */ 2 * /* num G2 */ 3;
        crate::circuits::num_inputs::<MNT4_753>(&[
            32,
            32,
            95,
            95,
            num_bits.div_ceil(8),
            95 * 2,
            /* y bits */ (5 + num_sub_inputs).div_ceil(8),
        ]) + vk_size
    }

    pub fn new(
        tree_level: usize,
        keys: VerifyingKeys,
        l_proof: Proof<MNT4_753>,
        r_proof: Proof<MNT4_753>,
        l_pk_node_hash: [u8; 32],
        r_pk_node_hash: [u8; 32],
        l_agg_pk_commitment: [u8; 95],
        r_agg_pk_commitment: [u8; 95],
        signer_bitmap_chunk: Vec<bool>,
    ) -> Self {
        Self {
            tree_level,
            vks_commitment: keys.commitment(),
            keys,
            l_proof,
            r_proof,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_pk_commitment,
            r_agg_pk_commitment,
            signer_bitmap_chunk,
        }
    }

    pub fn rand<R: Rng + ?Sized>(tree_level: usize, rng: &mut R) -> Self {
        let l_proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let r_proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let mut pk_node_hash = [0u8; 95];
        rng.fill_bytes(&mut pk_node_hash);

        let mut l_pk_node_hash = [0u8; 32];
        rng.fill_bytes(&mut l_pk_node_hash);
        let mut r_pk_node_hash = [0u8; 32];
        rng.fill_bytes(&mut r_pk_node_hash);

        let mut l_agg_pk_commitment = [0u8; 95];
        rng.fill_bytes(&mut l_agg_pk_commitment);

        let mut r_agg_pk_commitment = [0u8; 95];
        rng.fill_bytes(&mut r_agg_pk_commitment);

        let mut signer_bitmap_chunk =
            Vec::with_capacity(Policy::SLOTS as usize / 2_usize.pow(tree_level as u32));
        for _ in 0..Policy::SLOTS as usize / 2_usize.pow(tree_level as u32) {
            signer_bitmap_chunk.push(rng.gen());
        }

        let keys = VerifyingKeys::rand(rng);

        PKTreeNodeCircuit::new(
            tree_level,
            keys,
            l_proof,
            r_proof,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_pk_commitment,
            r_agg_pk_commitment,
            signer_bitmap_chunk,
        )
    }
}

impl ConstraintSynthesizer<MNT4Fq> for PKTreeNodeCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fq>) -> Result<(), SynthesisError> {
        // Allocate all the witnesses.
        let l_proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.l_proof))?;

        let r_proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.r_proof))?;

        // Allocate all the inputs.
        let l_pk_node_hash_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.l_pk_node_hash)?;

        let r_pk_node_hash_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.r_pk_node_hash)?;

        let l_agg_pk_commitment_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.l_agg_pk_commitment)?;

        let r_agg_pk_commitment_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.r_agg_pk_commitment)?;

        let signer_bitmap_chunk_bits =
            BitVec::<MNT4Fq>::new_input_vec(cs.clone(), &self.signer_bitmap_chunk[..])?;
        let signer_bitmap_bits = signer_bitmap_chunk_bits.0
            [..Policy::SLOTS as usize / 2_usize.pow(self.tree_level as u32)]
            .to_vec();
        let vks_commitment_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.vks_commitment)?;

        // Get merger vk.
        let child_vk = VerifyingKeyVar::new_compressed_input(cs.clone(), || {
            self.keys
                .get_key(CircuitId::PkTree(self.tree_level + 1))
                .ok_or(SynthesisError::AssignmentMissing)
        })?;

        // Split the signer's bitmap chunk into two, for the left and right child nodes.
        let (l_signer_bitmap_bits, r_signer_bitmap_bits) = signer_bitmap_bits
            .split_at(Policy::SLOTS as usize / 2_usize.pow((self.tree_level + 1) as u32));

        // Verify the ZK proof for the left child node.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&l_pk_node_hash_var)?;
        proof_inputs.push(&l_agg_pk_commitment_var)?;
        proof_inputs.push(l_signer_bitmap_bits)?;
        // We only pass the vks commitment to non-leaf nodes.
        if self.tree_level < 4 {
            proof_inputs.push(&vks_commitment_var)?;
        }

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &child_vk,
            &proof_inputs.into(),
            &l_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Verify the ZK proof for the right child node.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&r_pk_node_hash_var)?;
        proof_inputs.push(&r_agg_pk_commitment_var)?;
        proof_inputs.push(r_signer_bitmap_bits)?;
        // We only pass the vks commitment to non-leaf nodes.
        if self.tree_level < 4 {
            proof_inputs.push(&vks_commitment_var)?;
        }

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &child_vk,
            &proof_inputs.into(),
            &r_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
