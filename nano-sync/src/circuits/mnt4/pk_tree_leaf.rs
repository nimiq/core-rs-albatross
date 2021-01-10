use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var};
use ark_mnt6_753::{Fq, G1Projective, G2Projective};
use ark_r1cs_std::prelude::{
    AllocVar, Boolean, CondSelectGadget, CurveVar, EqGadget, ToBitsGadget,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::constants::{PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
use crate::gadgets::mnt4::{MerkleTreeGadget, PedersenHashGadget, SerializeGadget};
use crate::primitives::pedersen_generators;
use crate::utils::reverse_inner_byte_order;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is the leaf subcircuit of the PKTreeCircuit. This circuit main function is to process the
/// validator's public keys and "return" the aggregate public keys for the prepare and commit rounds
/// of the Macro Block. At a high-level, it divides all the computation into 2^n parts, where n
/// is the depth of the tree, so that each part uses only a manageable amount of memory and can be
/// run on consumer hardware.
/// It does this by forming a binary tree of recursive SNARKs. Each of the 2^n leaves receives
/// Merkle tree commitments to the public keys list and a commitment to the corresponding aggregate
/// public keys chunk (there are 2^n chunks, one for each leaf) in addition to the signer's
/// bitmaps and the position of the leaf node on the tree. Each of the leaves then checks that its
/// specific chunk of the public keys matches the corresponding chunk of the aggregated public keys.
/// All of the other upper levels of the recursive SNARK tree just verify SNARK proofs for its child
/// nodes and recursively aggregate the aggregate public keys chunks (no pun intended).
#[derive(Clone)]
pub struct PKTreeLeafCircuit {
    // Position of this leaf in the PKTree. Not an input to the SNARK circuit.
    position: usize,

    // Witnesses (private)
    pks: Vec<G2Projective>,
    pk_tree_nodes: Vec<G1Projective>,

    // Inputs (public)
    pk_tree_commitment: Vec<Fq>,
    agg_pk_commitment: Vec<Fq>,
    signer_bitmap: Fq,
    path: Fq,
}

impl PKTreeLeafCircuit {
    pub fn new(
        position: usize,
        pks: Vec<G2Projective>,
        pk_tree_nodes: Vec<G1Projective>,
        pk_tree_commitment: Vec<Fq>,
        agg_pk_commitment: Vec<Fq>,
        signer_bitmap: Fq,
        path: Fq,
    ) -> Self {
        Self {
            position,
            pks,
            pk_tree_nodes,
            pk_tree_commitment,
            agg_pk_commitment,
            signer_bitmap,
            path,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for PKTreeLeafCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let pedersen_generators_var =
            Vec::<G1Var>::new_constant(cs.clone(), pedersen_generators(195))?;

        // Allocate all the witnesses.
        next_cost_analysis!(cs, cost, || { "Alloc witnesses" });

        let pks_var = Vec::<G2Var>::new_witness(cs.clone(), || Ok(&self.pks[..]))?;

        let pk_tree_nodes_var =
            Vec::<G1Var>::new_witness(cs.clone(), || Ok(&self.pk_tree_nodes[..]))?;

        // Allocate all the inputs.
        next_cost_analysis!(cs, cost, || { "Alloc inputs" });

        let pk_tree_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.pk_tree_commitment[..]))?;

        let agg_pk_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.agg_pk_commitment[..]))?;

        let signer_bitmap_var = FqVar::new_input(cs.clone(), || Ok(&self.signer_bitmap))?;

        let path_var = FqVar::new_input(cs.clone(), || Ok(&self.path))?;

        // Process the inputs.
        next_cost_analysis!(cs, cost, || { "Process inputs" });

        // Convert the pk_tree_commitment and agg_pk_commitment from field elements to bits.
        let mut pk_tree_commitment_bits = vec![];

        for fp in pk_tree_commitment_var {
            let mut bits = fp.to_bits_le()?;
            pk_tree_commitment_bits.append(&mut bits);
        }

        let mut agg_pk_commitment_bits = vec![];

        for fp in agg_pk_commitment_var {
            let mut bits = fp.to_bits_le()?;
            agg_pk_commitment_bits.append(&mut bits);
        }

        // Only the first (in little-endian) VALIDATOR_SLOTS bits of the field element are used to
        // encode the signer bitmap. From these we get the part of the bitmap that is relevant to us.
        let chunk_start = VALIDATOR_SLOTS / PK_TREE_BREADTH * self.position;
        let chunk_end = VALIDATOR_SLOTS / PK_TREE_BREADTH * (1 + self.position);

        let signer_bitmap_var = signer_bitmap_var.to_bits_le()?[chunk_start..chunk_end].to_vec();

        // We take the path, turn it into bits and keep only the first (in little-endian)
        // PK_TREE_DEPTH bits. The resulting bits represent in effect the path in the PK Merkle tree
        // from our current position to the root. See MerkleTreeGadget for more details.
        let path_var = path_var.to_bits_le()?[..PK_TREE_DEPTH].to_vec();

        // Verify the Merkle proof for the public keys. To reiterate, this Merkle tree has 2^n
        // leaves (where n is the PK_TREE_DEPTH constant) and each of the leaves consists of several
        // public keys serialized and concatenated together. Each leaf contains exactly
        // VALIDATOR_SLOTS/2^n public keys, so that the entire Merkle tree contains all of the
        // public keys list.
        next_cost_analysis!(cs, cost, || { "Verify Merkle proof pks" });

        let mut bits: Vec<Boolean<MNT4Fr>> = vec![];

        for i in 0..self.pks.len() {
            bits.extend(SerializeGadget::serialize_g2(cs.clone(), &pks_var[i])?);
        }

        MerkleTreeGadget::verify(
            cs.clone(),
            &bits,
            &pk_tree_nodes_var,
            &path_var,
            &pk_tree_commitment_bits,
            &pedersen_generators_var,
        )?;

        // Calculate the aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate agg key" });

        let mut calculated_agg_pk = G2Var::zero();

        for (key, included) in pks_var.iter().zip(signer_bitmap_var.iter()) {
            // Calculate a new sum that includes the next public key.
            let new_sum = &calculated_agg_pk + key;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum =
                CondSelectGadget::conditionally_select(included, &new_sum, &calculated_agg_pk)?;

            calculated_agg_pk = cond_sum;
        }

        // Verifying aggregate public key. It checks that the calculated aggregate public key
        // is correct by comparing it with the aggregate public key commitment given as an input.
        next_cost_analysis!(cs, cost, || { "Verify agg pk" });

        let agg_pk_bits = SerializeGadget::serialize_g2(cs.clone(), &calculated_agg_pk)?;

        let pedersen_hash = PedersenHashGadget::evaluate(&agg_pk_bits, &pedersen_generators_var)?;

        let pedersen_bits = SerializeGadget::serialize_g1(cs.clone(), &pedersen_hash)?;

        let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

        agg_pk_commitment_bits.enforce_equal(&pedersen_bits)?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
