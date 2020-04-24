use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{G1Projective, G2Projective};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::{G1Gadget, G2Gadget};
use r1cs_std::prelude::*;

use crate::constants::{sum_generator_g1_mnt6, sum_generator_g2_mnt6, VALIDATOR_SLOTS};
use crate::gadgets::mnt4::{MerkleTreeGadget, SerializeGadget};
use crate::primitives::pedersen_generators;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

///
#[derive(Clone)]
pub struct PKTree3Circuit {
    // Private inputs
    pks: Vec<G2Projective>,
    pks_nodes: Vec<G1Projective>,
    prepare_agg_pk: G2Projective,
    prepare_agg_pk_nodes: Vec<G1Projective>,
    commit_agg_pk: G2Projective,
    commit_agg_pk_nodes: Vec<G1Projective>,

    // Public inputs
    pk_commitment: Vec<u8>,
    prepare_signer_bitmap: Vec<u8>,
    prepare_agg_pk_commitment: Vec<u8>,
    commit_signer_bitmap: Vec<u8>,
    commit_agg_pk_commitment: Vec<u8>,
    position: u8,
}

impl PKTree3Circuit {
    pub fn new(
        pks: Vec<G2Projective>,
        pks_nodes: Vec<G1Projective>,
        prepare_agg_pk: G2Projective,
        prepare_agg_pk_nodes: Vec<G1Projective>,
        commit_agg_pk: G2Projective,
        commit_agg_pk_nodes: Vec<G1Projective>,
        pk_commitment: Vec<u8>,
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
            prepare_agg_pk_nodes,
            commit_agg_pk,
            commit_agg_pk_nodes,
            pk_commitment,
            prepare_signer_bitmap,
            prepare_agg_pk_commitment,
            commit_signer_bitmap,
            commit_agg_pk_commitment,
            position,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for PKTree3Circuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let sum_generator_g1_var =
            G1Gadget::alloc_constant(cs.ns(|| "alloc sum generator g1"), &sum_generator_g1_mnt6())?;

        let sum_generator_g2_var =
            G2Gadget::alloc_constant(cs.ns(|| "alloc sum generator g2"), &sum_generator_g2_mnt6())?;

        // TODO: Calculate correct number of generators.
        let pedersen_generators = pedersen_generators(256);
        let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
        for i in 0..pedersen_generators.len() {
            pedersen_generators_var.push(G1Gadget::alloc_constant(
                cs.ns(|| format!("alloc pedersen_generators: generator {}", i)),
                &pedersen_generators[i],
            )?);
        }

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || { "Alloc private inputs" });

        let mut pks_var = Vec::new();
        for i in 0..self.pks.len() {
            pks_var.push(G2Gadget::alloc(
                cs.ns(|| format!("alloc public keys: key {}", i)),
                || Ok(&self.pks[i]),
            )?);
        }

        let mut pks_nodes_var = Vec::new();
        for i in 0..self.pks_nodes.len() {
            pks_nodes_var.push(G1Gadget::alloc(
                cs.ns(|| format!("alloc pks Merkle proof nodes: node {}", i)),
                || Ok(&self.pks_nodes[i]),
            )?);
        }

        let prepare_agg_pk_var =
            G2Gadget::alloc(cs.ns(|| "alloc prepare aggregate public key"), || {
                Ok(&self.prepare_agg_pk)
            })?;

        let mut prepare_agg_pk_nodes_var = Vec::new();
        for i in 0..self.prepare_agg_pk_nodes.len() {
            prepare_agg_pk_nodes_var.push(G1Gadget::alloc(
                cs.ns(|| format!("alloc prepare agg pk Merkle proof nodes: node {}", i)),
                || Ok(&self.prepare_agg_pk_nodes[i]),
            )?);
        }

        let commit_agg_pk_var =
            G2Gadget::alloc(cs.ns(|| "alloc commit aggregate public key"), || {
                Ok(&self.commit_agg_pk)
            })?;

        let mut commit_agg_pk_nodes_var = Vec::new();
        for i in 0..self.commit_agg_pk_nodes.len() {
            commit_agg_pk_nodes_var.push(G1Gadget::alloc(
                cs.ns(|| format!("alloc commit agg pk Merkle proof nodes: node {}", i)),
                || Ok(&self.commit_agg_pk_nodes[i]),
            )?);
        }

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc public keys commitment"),
            self.pk_commitment.as_ref(),
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

        let position_var = UInt8::alloc_input(cs.ns(|| "alloc position"), || Ok(self.position))?;

        // Process public inputs.
        next_cost_analysis!(cs, cost, || { "Process public inputs" });

        // Note: This assumes that there are 8 leaves in this tree.
        let chunk_start = VALIDATOR_SLOTS / 8 * (self.position as usize);
        let chunk_end = VALIDATOR_SLOTS / 8 * (1 + self.position as usize);

        let prepare_signer_bitmap_var =
            prepare_signer_bitmap_var.to_bits(cs.ns(|| "prepare signer bitmap to bits"))?;

        let prepare_signer_bitmap_var = prepare_signer_bitmap_var[chunk_start..chunk_end].to_vec();

        let commit_signer_bitmap_var =
            commit_signer_bitmap_var.to_bits(cs.ns(|| "commit signer bitmap to bits"))?;

        let commit_signer_bitmap_var = commit_signer_bitmap_var[chunk_start..chunk_end].to_vec();

        let mut path_var = position_var.to_bits(cs.ns(|| "get Merkle path from position"))?;

        path_var.truncate(3);

        // Verify the Merkle proof for the public keys.
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
            &sum_generator_g1_var,
        )?;

        // Verify the Merkle proof for the prepare aggregate public key.
        let bits = SerializeGadget::serialize_g2(
            cs.ns(|| "serialize prepare agg pk"),
            &prepare_agg_pk_var,
        )?;

        MerkleTreeGadget::verify(
            cs.ns(|| "verify Merkle proof for prepare agg pk"),
            &bits,
            &prepare_agg_pk_nodes_var,
            &path_var,
            &prepare_agg_pk_commitment_var,
            &pedersen_generators_var,
            &sum_generator_g1_var,
        )?;

        // Verify the Merkle proof for the commit aggregate public key.
        let bits =
            SerializeGadget::serialize_g2(cs.ns(|| "serialize commit agg pk"), &commit_agg_pk_var)?;

        MerkleTreeGadget::verify(
            cs.ns(|| "verify Merkle proof for commit agg pk"),
            &bits,
            &commit_agg_pk_nodes_var,
            &path_var,
            &commit_agg_pk_commitment_var,
            &pedersen_generators_var,
            &sum_generator_g1_var,
        )?;

        // Calculate the prepare aggregate public key.
        let mut sum = sum_generator_g2_var.clone();

        for (i, (key, included)) in pks_var
            .iter()
            .zip(prepare_signer_bitmap_var.iter())
            .enumerate()
        {
            // Calculate a new sum that includes the next public key.
            let new_sum = sum.add(cs.ns(|| format!("add public key, prepare {}", i)), key)?;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally add public key, prepare {}", i)),
                included,
                &new_sum,
                &sum,
            )?;

            sum = cond_sum;
        }

        let reference_prep_agg_pk = sum.sub(
            cs.ns(|| "finalize prepare aggregate public key"),
            &sum_generator_g2_var,
        )?;

        // Calculate the commit aggregate public key.
        let mut sum = sum_generator_g2_var.clone();

        for (i, (key, included)) in pks_var
            .iter()
            .zip(commit_signer_bitmap_var.iter())
            .enumerate()
        {
            // Calculate a new sum that includes the next public key.
            let new_sum = sum.add(cs.ns(|| format!("add public key, commit {}", i)), key)?;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally add public key, commit {}", i)),
                included,
                &new_sum,
                &sum,
            )?;

            sum = cond_sum;
        }

        let reference_comm_agg_pk = sum.sub(
            cs.ns(|| "finalize commit aggregate public key"),
            &sum_generator_g2_var,
        )?;

        // Check that both reference aggregate public keys match the ones given as inputs.
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
