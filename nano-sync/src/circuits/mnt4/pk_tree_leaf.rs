use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, G2Var};
use ark_mnt6_753::{G1Projective, G2Projective};
use ark_r1cs_std::prelude::*;
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
    // Witnesses (private)
    pks: Vec<G2Projective>,
    pks_nodes: Vec<G1Projective>,
    agg_pk: G2Projective,

    // Inputs (public)
    pks_commitment: Vec<u8>,
    signer_bitmap: Vec<u8>,
    agg_pk_commitment: Vec<u8>,
    position: u8,
}

impl PKTreeLeafCircuit {
    pub fn new(
        pks: Vec<G2Projective>,
        pks_nodes: Vec<G1Projective>,
        agg_pk: G2Projective,
        pks_commitment: Vec<u8>,
        signer_bitmap: Vec<u8>,
        agg_pk_commitment: Vec<u8>,
        position: u8,
    ) -> Self {
        Self {
            pks,
            pks_nodes,
            agg_pk,
            pks_commitment,
            signer_bitmap,
            agg_pk_commitment,
            position,
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

        let pks_nodes_var = Vec::<G1Var>::new_witness(cs.clone(), || Ok(&self.pks_nodes[..]))?;

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(self.agg_pk))?;

        // Allocate all the inputs.
        next_cost_analysis!(cs, cost, || { "Alloc inputs" });

        let pk_commitment_var = UInt8::new_input_vec(cs.clone(), &self.pks_commitment)?;

        let signer_bitmap_var = UInt8::new_input_vec(cs.clone(), &self.signer_bitmap)?;

        let agg_pk_commitment_var = UInt8::new_input_vec(cs.clone(), &self.agg_pk_commitment)?;

        let position_var = UInt8::new_input_vec(cs.clone(), &[self.position])?
            .pop()
            .unwrap();

        // Process inputs.
        next_cost_analysis!(cs, cost, || { "Process inputs" });

        let chunk_start = VALIDATOR_SLOTS / PK_TREE_BREADTH * (self.position as usize);
        let chunk_end = VALIDATOR_SLOTS / PK_TREE_BREADTH * (1 + self.position as usize);

        let signer_bitmap_var = signer_bitmap_var.to_bits_le()?;

        let signer_bitmap_var = signer_bitmap_var[chunk_start..chunk_end].to_vec();

        // We take the position, turn it into bits and keep only the first PK_TREE_DEPTH bits. The
        // resulting bits represent in effect the path in the PK Merkle tree from our current
        // position to the root. See MerkleTreeGadget for more details.
        let mut path_var = position_var.to_bits_le()?;

        path_var.truncate(PK_TREE_DEPTH);

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
            &pks_nodes_var,
            &path_var,
            &pk_commitment_var,
            &pedersen_generators_var,
        )?;

        // Verifying aggregate public key commitment. It just checks that the aggregate public key
        // given as a witness is correct by committing to it and comparing the result with the
        // aggregate public key commitment given as an input.
        next_cost_analysis!(cs, cost, || { "Verify agg pk commitment" });

        let agg_pk_bits = SerializeGadget::serialize_g2(cs.clone(), &agg_pk_var)?;

        let pedersen_hash = PedersenHashGadget::evaluate(&agg_pk_bits, &pedersen_generators_var)?;

        let pedersen_bits = SerializeGadget::serialize_g1(cs.clone(), &pedersen_hash)?;

        let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

        let mut reference_commitment = Vec::new();

        // Convert pedersen_bits, which is a Vector of Booleans, into a Vector of UInt8s.
        for i in 0..pedersen_bits.len() / 8 {
            reference_commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
        }

        agg_pk_commitment_var.enforce_equal(&reference_commitment)?;

        // Calculate the aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate agg key" });

        let mut reference_agg_pk = G2Var::zero();

        for (key, included) in pks_var.iter().zip(signer_bitmap_var.iter()) {
            // Calculate a new sum that includes the next public key.
            let new_sum = &reference_agg_pk + key;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum =
                CondSelectGadget::conditionally_select(included, &new_sum, &reference_agg_pk)?;

            reference_agg_pk = cond_sum;
        }

        // Check that the reference aggregate public key matches the one given as an input.
        next_cost_analysis!(cs, cost, || { "Verify agg key" });

        agg_pk_var.enforce_equal(&reference_agg_pk)?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
