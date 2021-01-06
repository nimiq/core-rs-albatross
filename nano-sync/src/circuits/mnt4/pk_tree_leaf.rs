use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::{G1Projective, G2Projective};
use ark_r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use ark_r1cs_std::mnt6_753::{G1Gadget, G2Gadget};
use ark_r1cs_std::prelude::*;

use crate::constants::{sum_generator_g2_mnt6, PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
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
    // Private inputs
    pks: Vec<G2Projective>,
    pks_nodes: Vec<G1Projective>,
    prepare_agg_pk: G2Projective,
    commit_agg_pk: G2Projective,

    // Public inputs
    pks_commitment: Vec<u8>,
    prepare_signer_bitmap: Vec<u8>,
    prepare_agg_pk_commitment: Vec<u8>,
    commit_signer_bitmap: Vec<u8>,
    commit_agg_pk_commitment: Vec<u8>,
    position: u8,
}

impl PKTreeLeafCircuit {
    pub fn new(
        pks: Vec<G2Projective>,
        pks_nodes: Vec<G1Projective>,
        prepare_agg_pk: G2Projective,
        commit_agg_pk: G2Projective,
        pks_commitment: Vec<u8>,
        prepare_signer_bitmap: Vec<u8>,
        prepare_agg_pk_commitment: Vec<u8>,
        commit_signer_bitmap: Vec<u8>,
        commit_agg_pk_commitment: Vec<u8>,
        position: u8,
    ) -> Self {
        Self {
            pks,
            pks_nodes,
            prepare_agg_pk,
            commit_agg_pk,
            pks_commitment,
            prepare_signer_bitmap,
            prepare_agg_pk_commitment,
            commit_signer_bitmap,
            commit_agg_pk_commitment,
            position,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for PKTreeLeafCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let sum_generator_g2_var =
            G2Gadget::alloc_constant(cs.ns(|| "alloc sum generator g2"), &sum_generator_g2_mnt6())?;

        let pedersen_generators_var = Vec::<G1Gadget>::alloc_constant(
            cs.ns(|| "alloc pedersen_generators"),
            pedersen_generators(195),
        )?;

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || { "Alloc private inputs" });

        let pks_var = Vec::<G2Gadget>::alloc(cs.ns(|| "alloc public keys"), || Ok(&self.pks[..]))?;

        let pks_nodes_var =
            Vec::<G1Gadget>::alloc(cs.ns(|| "alloc pks Merkle proof nodes"), || {
                Ok(&self.pks_nodes[..])
            })?;

        let prepare_agg_pk_var =
            G2Gadget::alloc(cs.ns(|| "alloc prepare aggregate public key"), || {
                Ok(&self.prepare_agg_pk)
            })?;

        let commit_agg_pk_var =
            G2Gadget::alloc(cs.ns(|| "alloc commit aggregate public key"), || {
                Ok(&self.commit_agg_pk)
            })?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc public keys commitment"),
            self.pks_commitment.as_ref(),
        )?;

        let prepare_signer_bitmap_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc prepare signer bitmap"),
            self.prepare_signer_bitmap.as_ref(),
        )?;

        let prepare_agg_pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc prepare aggregate pk commitment"),
            self.prepare_agg_pk_commitment.as_ref(),
        )?;

        let commit_signer_bitmap_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc commit signer bitmap"),
            self.commit_signer_bitmap.as_ref(),
        )?;

        let commit_agg_pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc commit aggregate pk commitment"),
            self.commit_agg_pk_commitment.as_ref(),
        )?;

        let position_var = UInt8::alloc_input_vec(cs.ns(|| "alloc position"), &[self.position])?
            .pop()
            .unwrap();

        // Process public inputs.
        next_cost_analysis!(cs, cost, || { "Process public inputs" });

        let chunk_start = VALIDATOR_SLOTS / PK_TREE_BREADTH * (self.position as usize);
        let chunk_end = VALIDATOR_SLOTS / PK_TREE_BREADTH * (1 + self.position as usize);

        let prepare_signer_bitmap_var =
            prepare_signer_bitmap_var.to_bits(cs.ns(|| "prepare signer bitmap to bits"))?;

        let prepare_signer_bitmap_var = prepare_signer_bitmap_var[chunk_start..chunk_end].to_vec();

        let commit_signer_bitmap_var =
            commit_signer_bitmap_var.to_bits(cs.ns(|| "commit signer bitmap to bits"))?;

        let commit_signer_bitmap_var = commit_signer_bitmap_var[chunk_start..chunk_end].to_vec();

        // We take the position, turn it into bits and keep only the first PK_TREE_DEPTH bits. The
        // resulting bits represent in effect the path in the PK Merkle tree from our current
        // position to the root. See MerkleTreeGadget for more details.
        let mut path_var = position_var.into_bits_le();

        path_var.truncate(PK_TREE_DEPTH);

        // Verify the Merkle proof for the public keys. To reiterate, this Merkle tree has 2^n
        // leaves (where n is the PK_TREE_DEPTH constant) and each of the leaves consists of several
        // public keys serialized and concatenated together. Each leaf contains exactly
        // VALIDATOR_SLOTS/2^n public keys, so that the entire Merkle tree contains all of the
        // public keys list.
        next_cost_analysis!(cs, cost, || { "Verify Merkle proof pks" });

        let mut bits: Vec<Boolean> = vec![];

        for i in 0..self.pks.len() {
            bits.extend(SerializeGadget::serialize_g2(
                cs.ns(|| format!("serialize pks: {}", i)),
                &pks_var[i],
            )?);
        }

        MerkleTreeGadget::verify(
            cs.ns(|| "verify Merkle proof for pks"),
            &bits,
            &pks_nodes_var,
            &path_var,
            &pk_commitment_var,
            &pedersen_generators_var,
        )?;

        // Verifying prepare aggregate public key commitment. It just checks that the prepare
        // aggregate public key given as private input is correct by committing to it and comparing
        // the result with the prepare aggregate public key commitment given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify prepare agg pk commitment" });

        let prepare_agg_pk_bits = SerializeGadget::serialize_g2(
            cs.ns(|| "serialize prepare agg pk"),
            &prepare_agg_pk_var,
        )?;

        let pedersen_hash = PedersenHashGadget::evaluate(
            cs.ns(|| "reference prepare agg pk commitment"),
            &prepare_agg_pk_bits,
            &pedersen_generators_var,
        )?;

        let pedersen_bits = SerializeGadget::serialize_g1(
            cs.ns(|| "serialize prepare pedersen hash"),
            &pedersen_hash,
        )?;

        let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

        let mut reference_commitment = Vec::new();

        // Convert pedersen_bits, which is a Vector of Booleans, into a Vector of UInt8s.
        for i in 0..pedersen_bits.len() / 8 {
            reference_commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
        }

        prepare_agg_pk_commitment_var.enforce_equal(
            cs.ns(|| "prepare agg pk commitment == reference commitment"),
            &reference_commitment,
        )?;

        // Verifying commit aggregate public key commitment. It just checks that the commit
        // aggregate public key given as private input is correct by committing to it and comparing
        // the result with the commit aggregate public key commitment given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify commit agg pk commitment" });

        let commit_agg_pk_bits =
            SerializeGadget::serialize_g2(cs.ns(|| "serialize commit agg pk"), &commit_agg_pk_var)?;

        let pedersen_hash = PedersenHashGadget::evaluate(
            cs.ns(|| "reference commit agg pk commitment"),
            &commit_agg_pk_bits,
            &pedersen_generators_var,
        )?;

        let pedersen_bits = SerializeGadget::serialize_g1(
            cs.ns(|| "serialize commit pedersen hash"),
            &pedersen_hash,
        )?;

        let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

        let mut reference_commitment = Vec::new();

        // Convert pedersen_bits, which is a Vector of Booleans, into a Vector of UInt8s.
        for i in 0..pedersen_bits.len() / 8 {
            reference_commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
        }

        commit_agg_pk_commitment_var.enforce_equal(
            cs.ns(|| "commit agg pk commitment == reference commitment"),
            &reference_commitment,
        )?;

        // Calculate the prepare aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate prepare agg key" });

        let mut reference_prep_agg_pk = sum_generator_g2_var.clone();

        for (i, (key, included)) in pks_var
            .iter()
            .zip(prepare_signer_bitmap_var.iter())
            .enumerate()
        {
            // Calculate a new sum that includes the next public key.
            let new_sum = reference_prep_agg_pk
                .add(cs.ns(|| format!("add public key, prepare {}", i)), key)?;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally add public key, prepare {}", i)),
                included,
                &new_sum,
                &reference_prep_agg_pk,
            )?;

            reference_prep_agg_pk = cond_sum;
        }

        // Calculate the commit aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate commit agg key" });

        let mut reference_comm_agg_pk = sum_generator_g2_var;

        for (i, (key, included)) in pks_var
            .iter()
            .zip(commit_signer_bitmap_var.iter())
            .enumerate()
        {
            // Calculate a new sum that includes the next public key.
            let new_sum = reference_comm_agg_pk
                .add(cs.ns(|| format!("add public key, commit {}", i)), key)?;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally add public key, commit {}", i)),
                included,
                &new_sum,
                &reference_comm_agg_pk,
            )?;

            reference_comm_agg_pk = cond_sum;
        }

        // Check that both reference aggregate public keys match the ones given as inputs.
        next_cost_analysis!(cs, cost, || { "Verify prepare and commit agg key" });

        prepare_agg_pk_var.enforce_equal(
            cs.ns(|| "verify equality prepare agg pk"),
            &reference_prep_agg_pk,
        )?;

        commit_agg_pk_var.enforce_equal(
            cs.ns(|| "verify equality commit agg pk"),
            &reference_comm_agg_pk,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
