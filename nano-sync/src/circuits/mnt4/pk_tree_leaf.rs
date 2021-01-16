use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var};
use ark_mnt6_753::{Fq, G1Projective, G2Projective};
use ark_r1cs_std::prelude::{AllocVar, Boolean, CondSelectGadget, CurveVar, EqGadget};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::constants::{PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
use crate::gadgets::mnt4::{MerkleTreeGadget, PedersenHashGadget, SerializeGadget};
use crate::primitives::pedersen_generators;
use crate::utils::{reverse_inner_byte_order, unpack_inputs};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is the leaf subcircuit of the PKTreeCircuit. This circuit main function is to process the
/// validator's public keys and "return" the aggregate public keys for the Macro Block. At a
/// high-level, it divides all the computation into 2^n parts, where n is the depth of the tree, so
/// that each part uses only a manageable amount of memory and can be run on consumer hardware.
/// It does this by forming a binary tree of recursive SNARKs. Each of the 2^n leaves receives
/// Merkle tree commitments to the public keys list and a commitment to the corresponding aggregate
/// public key chunk (there are 2^n chunks, one for each leaf) in addition to the signer's
/// bitmaps and the position of the leaf node on the tree. Each of the leaves then checks that its
/// specific chunk of the public keys matches the corresponding chunk of the aggregated public key.
/// All of the other upper levels of the recursive SNARK tree just verify SNARK proofs for its child
/// nodes and recursively aggregate the aggregate public key chunks (no pun intended).
/// At a lower-level, this circuit does two things:
///     1. That the public keys given as witness are a leaf, in a specific position, of the Merkle
///        tree of public keys. The Merkle tree root and the position are given as inputs and the
///        Merkle proof is given as a witness.
///     2. That the public keys given as witness, when aggregated according to the signer's bitmap
///        (given as an input), match the aggregated public key commitment (also given as an input).
#[derive(Clone)]
pub struct PKTreeLeafCircuit {
    // Position of this leaf in the PKTree. Not an input to the SNARK circuit.
    position: usize,

    // Witnesses (private)
    pks: Vec<G2Projective>,
    pk_tree_nodes: Vec<G1Projective>,

    // Inputs (public)
    // Our inputs are always vectors of booleans (semantically), so that they are consistent across
    // the different elliptic curves that we use. However, for compactness, we represent them as
    // field elements. Both of the curves that we use have a modulus of 753 bits and a capacity
    // of 752 bits. So, the first 752 bits (in little-endian) of each field element is data, and the
    // last bit is always set to zero.
    pk_tree_root: Vec<Fq>,
    agg_pk_commitment: Vec<Fq>,
    signer_bitmap: Fq,
    path: Fq,
}

impl PKTreeLeafCircuit {
    pub fn new(
        position: usize,
        pks: Vec<G2Projective>,
        pk_tree_nodes: Vec<G1Projective>,
        pk_tree_root: Vec<Fq>,
        agg_pk_commitment: Vec<Fq>,
        signer_bitmap: Fq,
        path: Fq,
    ) -> Self {
        Self {
            position,
            pks,
            pk_tree_nodes,
            pk_tree_root,
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

        let pk_tree_root_var = Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.pk_tree_root[..]))?;

        let agg_pk_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.agg_pk_commitment[..]))?;

        let signer_bitmap_var = FqVar::new_input(cs.clone(), || Ok(&self.signer_bitmap))?;

        let path_var = FqVar::new_input(cs.clone(), || Ok(&self.path))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        next_cost_analysis!(cs, cost, || { "Unpack inputs" });

        let pk_tree_root_bits = unpack_inputs(pk_tree_root_var)?[..760].to_vec();

        let agg_pk_commitment_bits = unpack_inputs(agg_pk_commitment_var)?[..760].to_vec();

        let signer_bitmap_bits =
            unpack_inputs(vec![signer_bitmap_var])?[..VALIDATOR_SLOTS].to_vec();

        let path_bits = unpack_inputs(vec![path_var])?[..PK_TREE_DEPTH].to_vec();

        // Get the part of the signer's bitmap that is relevant for this position.
        let chunk_start = VALIDATOR_SLOTS / PK_TREE_BREADTH * self.position;

        let chunk_end = VALIDATOR_SLOTS / PK_TREE_BREADTH * (1 + self.position);

        let signer_bitmap_chunk = signer_bitmap_bits[chunk_start..chunk_end].to_vec();

        // Verify the Merkle proof for the public keys. To reiterate, this Merkle tree has 2^n
        // leaves (where n is the PK_TREE_DEPTH constant) and each of the leaves consists of several
        // public keys serialized and concatenated together. Each leaf contains exactly
        // VALIDATOR_SLOTS/2^n public keys, so that the entire Merkle tree contains all of the
        // public keys list.
        next_cost_analysis!(cs, cost, || { "Verify Merkle proof pks" });

        let mut bits = vec![];

        for i in 0..self.pks.len() {
            bits.extend(SerializeGadget::serialize_g2(cs.clone(), &pks_var[i])?);
        }

        MerkleTreeGadget::verify(
            cs.clone(),
            &bits,
            &pk_tree_nodes_var,
            &path_bits,
            &pk_tree_root_bits,
            &pedersen_generators_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Calculate the aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate agg key" });

        let mut calculated_agg_pk = G2Var::zero();

        for (pk, included) in pks_var.iter().zip(signer_bitmap_chunk.iter()) {
            // Calculate a new sum that includes the next public key.
            let new_sum = &calculated_agg_pk + pk;

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
