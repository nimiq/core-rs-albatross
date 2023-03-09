use ark_crypto_primitives::snark::{BooleanInputVar, FromFieldElementsGadget, SNARKGadget};
use ark_ff::ToConstraintField;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt4_753::{
    constraints::{FqVar, PairingVar},
    Fq as MNT4Fq, MNT4_753,
};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    ToBitsGadget, ToConstraintFieldGadget,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::PK_TREE_DEPTH;

use crate::{gadgets::bits::BitVec, utils::bits_to_bytes};

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
    vk_child: VerifyingKey<MNT4_753>,

    // Witnesses (private)
    left_proof: Proof<MNT4_753>,
    right_proof: Proof<MNT4_753>,

    // Inputs (public)
    pk_tree_root: [u8; 95],
    left_agg_pk_commitment: [u8; 95],
    right_agg_pk_commitment: [u8; 95],
    signer_bitmap_chunk: Vec<bool>,
    path: u8,
}

impl PKTreeNodeCircuit {
    pub fn new(
        tree_level: usize,
        vk_child: VerifyingKey<MNT4_753>,
        left_proof: Proof<MNT4_753>,
        right_proof: Proof<MNT4_753>,
        pk_tree_root: [u8; 95],
        left_agg_pk_commitment: [u8; 95],
        right_agg_pk_commitment: [u8; 95],
        signer_bitmap_chunk: Vec<bool>,
        path: u8,
    ) -> Self {
        Self {
            tree_level,
            vk_child,
            left_proof,
            right_proof,
            pk_tree_root,
            left_agg_pk_commitment,
            right_agg_pk_commitment,
            signer_bitmap_chunk,
            path,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fq> for PKTreeNodeCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fq>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let vk_child_var =
            VerifyingKeyVar::<MNT4_753, PairingVar>::new_constant(cs.clone(), &self.vk_child)?;

        // Allocate all the witnesses.
        let left_proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.left_proof))?;

        let right_proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.right_proof))?;

        // Allocate all the inputs.
        // Since we're only passing it through, we directly allocate them as FqVars.
        let pk_tree_root_var = Vec::<FqVar>::new_input(cs.clone(), || {
            self.pk_tree_root
                .to_field_elements()
                .ok_or(SynthesisError::AssignmentMissing)
        })?;

        let mut left_agg_pk_commitment_var = Vec::<FqVar>::new_input(cs.clone(), || {
            self.left_agg_pk_commitment
                .to_field_elements()
                .ok_or(SynthesisError::AssignmentMissing)
        })?;

        let mut right_agg_pk_commitment_var = Vec::<FqVar>::new_input(cs.clone(), || {
            self.right_agg_pk_commitment
                .to_field_elements()
                .ok_or(SynthesisError::AssignmentMissing)
        })?;

        let signer_bitmap_chunk_bits =
            BitVec::<MNT4Fq>::new_input_vec(cs.clone(), &self.signer_bitmap_chunk[..])?;
        let signer_bitmap_bits = signer_bitmap_chunk_bits.0
            [..Policy::SLOTS as usize / 2_usize.pow(self.tree_level as u32)]
            .to_vec();

        let path_var = FqVar::new_input(cs, || Ok(MNT4Fq::from(self.path)))?;

        let mut path_bits = path_var.to_bits_le()?[..PK_TREE_DEPTH].to_vec();

        // Calculate the path for the left and right child nodes. Given the current position P,
        // the left position L and the right position R are given as:
        //    L = 2 * P
        //    R = 2 * P + 1
        // For efficiency reasons, we actually calculate the path using bit manipulation.

        // Calculate P >> 1, which is equivalent to calculating 2 * P (in little-endian).
        path_bits.pop();
        path_bits.insert(0, Boolean::Constant(false));
        let left_path = path_bits.clone();

        // path_bits is currently P >> 1 = L. Calculate L & 1, which is equivalent to L + 1 (in little-endian).
        let _ = path_bits.remove(0);
        path_bits.insert(0, Boolean::Constant(true));
        let right_path = path_bits;

        // Split the signer's bitmap chunk into two, for the left and right child nodes.
        let (left_signer_bitmap_bits, right_signer_bitmap_bits) = signer_bitmap_bits
            .split_at(Policy::SLOTS as usize / 2_usize.pow((self.tree_level + 1) as u32));

        let mut left_signer_bitmap_var =
            bits_to_bytes(left_signer_bitmap_bits).to_constraint_field()?;
        let mut right_signer_bitmap_var =
            bits_to_bytes(right_signer_bitmap_bits).to_constraint_field()?;
        let mut left_path_var = bits_to_bytes(&left_path).to_constraint_field()?;
        let mut right_path_var = bits_to_bytes(&right_path).to_constraint_field()?;

        // Verify the ZK proof for the left child node.
        let mut proof_inputs = pk_tree_root_var.clone();

        proof_inputs.append(&mut left_agg_pk_commitment_var);

        proof_inputs.append(&mut left_signer_bitmap_var);

        proof_inputs.append(&mut left_path_var);

        let input_var = BooleanInputVar::from_field_elements(&proof_inputs)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_child_var,
            &input_var,
            &left_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Verify the ZK proof for the right child node.
        let mut proof_inputs = pk_tree_root_var;

        proof_inputs.append(&mut right_agg_pk_commitment_var);

        proof_inputs.append(&mut right_signer_bitmap_var);

        proof_inputs.append(&mut right_path_var);

        let input_var = BooleanInputVar::from_field_elements(&proof_inputs)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_child_var,
            &input_var,
            &right_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
