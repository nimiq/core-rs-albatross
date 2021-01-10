use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var, PairingVar};
use ark_mnt6_753::{Fq, Fr as MNT6Fr, G2Projective, MNT6_753};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_serialize::CanonicalDeserialize;
use std::fs::File;
use std::marker::PhantomData;

use crate::constants::PK_TREE_DEPTH;
use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};
use crate::primitives::pedersen_generators;
use crate::utils::reverse_inner_byte_order;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};
use ark_crypto_primitives::SNARKGadget;
use ark_r1cs_std::prelude::{AllocVar, Boolean, CurveVar, EqGadget, UInt8};
use ark_r1cs_std::ToBitsGadget;

/// This is the node subcircuit of the PKTreeCircuit. See PKTreeLeafCircuit for more details.
/// It is different from the other node subcircuit on the MNT6 curve in that it does recalculate
/// the aggregate public key commitments.
#[derive(Clone)]
pub struct PKTreeNodeCircuit {
    // Path to the verifying key file. Not an input to the SNARK circuit.
    vk_file: &'static str,

    // Witnesses (private)
    left_proof: Proof<MNT6_753>,
    right_proof: Proof<MNT6_753>,
    agg_pk_chunks: Vec<G2Projective>,

    // Inputs (public)
    pk_tree_commitment: Vec<Fq>,
    agg_pk_commitment: Vec<Fq>,
    signer_bitmap: Fq,
    path: Fq,
}

impl PKTreeNodeCircuit {
    pub fn new(
        vk_file: &'static str,
        left_proof: Proof<MNT6_753>,
        right_proof: Proof<MNT6_753>,
        agg_pk_chunks: Vec<G2Projective>,
        pk_tree_commitment: Vec<Fq>,
        agg_pk_commitment: Vec<Fq>,
        signer_bitmap: Fq,
        path: Fq,
    ) -> Self {
        Self {
            vk_file,
            left_proof,
            right_proof,
            agg_pk_chunks,
            pk_tree_commitment,
            agg_pk_commitment,
            signer_bitmap,
            path,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for PKTreeNodeCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Load the verifying key from file.
        let mut file = File::open(format!("verifying_keys/{}", &self.vk_file)).unwrap();

        let vk_child = VerifyingKey::deserialize(&mut file).unwrap();

        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let pedersen_generators_var =
            Vec::<G1Var>::new_constant(cs.clone(), pedersen_generators(5))?;

        let vk_child_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(cs.clone(), vk_child)?;

        // Allocate all the witnesses.
        next_cost_analysis!(cs, cost, || { "Alloc witnesses" });

        let left_proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.left_proof))?;

        let right_proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.right_proof))?;

        let agg_pk_chunks_var =
            Vec::<G2Var>::new_witness(cs.clone(), || Ok(&self.agg_pk_chunks[..]))?;

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

        // Calculating the aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate agg pk" });

        let mut agg_pk = G2Var::zero();

        for key in &agg_pk_chunks_var {
            agg_pk += key;
        }

        // Verifying aggregate public key commitment. It just checks that the calculated aggregate
        // public key is correct by comparing it with the aggregate public key commitment given as
        // an input.
        next_cost_analysis!(cs, cost, || { "Verify agg pk" });

        let agg_pk_bits = SerializeGadget::serialize_g2(cs.clone(), &agg_pk)?;

        let pedersen_hash = PedersenHashGadget::evaluate(&agg_pk_bits, &pedersen_generators_var)?;

        let pedersen_bits = SerializeGadget::serialize_g1(cs.clone(), &pedersen_hash)?;

        let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

        agg_pk_commitment_bits.enforce_equal(&pedersen_bits)?;

        // Calculating the commitments to each of the aggregate public keys chunks. These
        // will be given as input to the SNARK circuits lower on the tree.
        next_cost_analysis!(cs, cost, || { "Calculate agg pk chunks commitments" });

        let mut agg_pk_chunks_commitments = Vec::new();

        for chunk in &agg_pk_chunks_var {
            let chunk_bits = SerializeGadget::serialize_g2(cs.clone(), chunk)?;

            let pedersen_hash =
                PedersenHashGadget::evaluate(&chunk_bits, &pedersen_generators_var)?;

            let pedersen_bits = SerializeGadget::serialize_g1(cs.clone(), &pedersen_hash)?;

            let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

            agg_pk_chunks_commitments.push(pedersen_bits);
        }

        // Calculate the path for the left and right child nodes. Given the current position P,
        // the left position L and the right position R are given as:
        //    L = 2 * P
        //    R = 2 * P + 1
        // For efficiency reasons, we actually calculate the path using bit manipulation.
        next_cost_analysis!(cs, cost, || { "Calculate paths" });

        // We take the path, turn it into bits and keep only the first (in little-endian)
        // PK_TREE_DEPTH bits.
        let mut path_bits = path_var.to_bits_le()?[..PK_TREE_DEPTH].to_vec();

        // Calculate P >> 1, which is equivalent to calculating 2 * P (in little-endian).
        path_bits.pop();
        path_bits.insert(0, Boolean::Constant(false));
        let left_path = path_bits.clone();

        // path_bits is currently P >> 1 = L. Calculate L & 1, which is equivalent to L + 1 (in little-endian).
        path_bits.remove(0);
        path_bits.insert(0, Boolean::Constant(true));
        let right_path = path_bits;

        // Verify the ZK proof for the left child node.
        next_cost_analysis!(cs, cost, || { "Verify left ZK proof" });
        let mut proof_inputs =
            RecursiveInputGadget::to_field_elements::<MNT4Fr, MNT6Fr>(&pk_commitment_var)?;

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<
            MNT4Fr,
            MNT6Fr,
        >(&signer_bitmap_var)?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<
            MNT4Fr,
            MNT6Fr,
        >(&agg_pk_chunks_commitments[0])?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<
            MNT4Fr,
            MNT6Fr,
        >(&agg_pk_chunks_commitments[1])?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<
            MNT4Fr,
            MNT6Fr,
        >(&[left_position])?);

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_child_var,
            Groth16VerifierGadget::InputVar::new(proof_inputs),
            &left_proof_var,
        )?;

        // // Verify the ZK proof for the right child node.
        // next_cost_analysis!(cs, cost, || { "Verify right ZK proof" });
        // let mut proof_inputs = RecursiveInputGadget::to_field_elements::<Fr>(&pk_commitment_var)?;
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &prepare_signer_bitmap_var,
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &prepare_agg_pk_chunks_commitments[2],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &prepare_agg_pk_chunks_commitments[3],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &commit_signer_bitmap_var,
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &commit_agg_pk_chunks_commitments[2],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &commit_agg_pk_chunks_commitments[3],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(&[
        //     right_position,
        // ])?);
        //
        // <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem<SubCircuit>, Fq>>::check_verify(
        //     cs.ns(|| "verify right groth16 proof"),
        //     &vk_child_var,
        //     proof_inputs.iter(),
        //     &right_proof_var,
        // )?;
        //
        // end_cost_analysis!(cs, cost);

        Ok(())
    }
}
