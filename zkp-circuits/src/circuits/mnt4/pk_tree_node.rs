use ark_crypto_primitives::snark::{BooleanInputVar, SNARKGadget};
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt4_753::{
    constraints::{FqVar, PairingVar},
    Fq as MNT4Fq, MNT4_753,
};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::PK_TREE_DEPTH;

use crate::utils::{prepare_inputs, unpack_inputs};

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
    // Our inputs are always vectors of booleans (semantically), so that they are consistent across
    // the different elliptic curves that we use. However, for compactness, we represent them as
    // field elements. Both of the curves that we use have a modulus of 753 bits and a capacity
    // of 752 bits. So, the first 752 bits (in little-endian) of each field element is data, and the
    // last bit is always set to zero.
    pk_tree_root: Vec<MNT4Fq>,
    left_agg_pk_commitment: Vec<MNT4Fq>,
    right_agg_pk_commitment: Vec<MNT4Fq>,
    signer_bitmap_chunk: MNT4Fq,
    path: MNT4Fq,
}

impl PKTreeNodeCircuit {
    pub fn new(
        tree_level: usize,
        vk_child: VerifyingKey<MNT4_753>,
        left_proof: Proof<MNT4_753>,
        right_proof: Proof<MNT4_753>,
        pk_tree_root: Vec<MNT4Fq>,
        left_agg_pk_commitment: Vec<MNT4Fq>,
        right_agg_pk_commitment: Vec<MNT4Fq>,
        signer_bitmap_chunk: MNT4Fq,
        path: MNT4Fq,
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
        let pk_tree_root_var = Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.pk_tree_root[..]))?;

        let left_agg_pk_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.left_agg_pk_commitment[..]))?;

        let right_agg_pk_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.right_agg_pk_commitment[..]))?;

        let signer_bitmap_chunk_var =
            FqVar::new_input(cs.clone(), || Ok(&self.signer_bitmap_chunk))?;

        let path_var = FqVar::new_input(cs, || Ok(&self.path))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        let pk_tree_root_bits = unpack_inputs(pk_tree_root_var)?[..760].to_vec();

        let left_agg_pk_commitment_bits =
            unpack_inputs(left_agg_pk_commitment_var)?[..760].to_vec();

        let right_agg_pk_commitment_bits =
            unpack_inputs(right_agg_pk_commitment_var)?[..760].to_vec();

        let signer_bitmap_bits = unpack_inputs(vec![signer_bitmap_chunk_var])?
            [..Policy::SLOTS as usize / 2_usize.pow(self.tree_level as u32)]
            .to_vec();

        let mut path_bits = unpack_inputs(vec![path_var])?[..PK_TREE_DEPTH].to_vec();

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

        // Verify the ZK proof for the left child node.
        let mut proof_inputs = prepare_inputs(pk_tree_root_bits.clone());

        proof_inputs.append(&mut prepare_inputs(left_agg_pk_commitment_bits));

        proof_inputs.append(&mut prepare_inputs(left_signer_bitmap_bits.to_vec()));

        proof_inputs.append(&mut prepare_inputs(left_path));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_child_var,
            &input_var,
            &left_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Verify the ZK proof for the right child node.
        let mut proof_inputs = prepare_inputs(pk_tree_root_bits);

        proof_inputs.append(&mut prepare_inputs(right_agg_pk_commitment_bits));

        proof_inputs.append(&mut prepare_inputs(right_signer_bitmap_bits.to_vec()));

        proof_inputs.append(&mut prepare_inputs(right_path));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_child_var,
            &input_var,
            &right_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
