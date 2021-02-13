use ark_crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use ark_crypto_primitives::NIZKVerifierGadget;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::{Fq, Fr, G2Projective, MNT6_753};
use ark_r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use ark_r1cs_std::mnt6_753::{G1Gadget, G2Gadget, PairingGadget};
use ark_r1cs_std::prelude::*;
use ark_serialize::CanonicalDeserialize;
use std::fs::File;
use std::marker::PhantomData;

use crate::constants::sum_generator_g2_mnt6;
use crate::gadgets::input::RecursiveInputGadget;
use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};
use crate::primitives::pedersen_generators;
use crate::utils::reverse_inner_byte_order;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience.
type TheProofSystem<T> = Groth16<MNT6_753, T, Fr>;
type TheProofGadget = ProofGadget<MNT6_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT6_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT6_753, Fq, PairingGadget>;

/// This is the node subcircuit of the PKTreeCircuit. See PKTreeLeafCircuit for more details.
/// It is different from the other node subcircuit on the MNT6 curve in that it does recalculate
/// the aggregate public key commitments.
#[derive(Clone)]
pub struct PKTreeNodeCircuit<SubCircuit> {
    _subcircuit: PhantomData<SubCircuit>,

    // Path to the verifying key file. Not an input to the SNARK circuit.
    vk_file: &'static str,

    // Private inputs
    left_proof: Proof<MNT6_753>,
    right_proof: Proof<MNT6_753>,
    prepare_agg_pk_chunks: Vec<G2Projective>,
    commit_agg_pk_chunks: Vec<G2Projective>,

    // Public inputs
    pks_commitment: Vec<u8>,
    prepare_signer_bitmap: Vec<u8>,
    prepare_agg_pk_commitment: Vec<u8>,
    commit_signer_bitmap: Vec<u8>,
    commit_agg_pk_commitment: Vec<u8>,
    position: u8,
}

impl<SubCircuit> PKTreeNodeCircuit<SubCircuit> {
    pub fn new(
        vk_file: &'static str,
        left_proof: Proof<MNT6_753>,
        right_proof: Proof<MNT6_753>,
        prepare_agg_pk_chunks: Vec<G2Projective>,
        commit_agg_pk_chunks: Vec<G2Projective>,
        pks_commitment: Vec<u8>,
        prepare_signer_bitmap: Vec<u8>,
        prepare_agg_pk_commitment: Vec<u8>,
        commit_signer_bitmap: Vec<u8>,
        commit_agg_pk_commitment: Vec<u8>,
        position: u8,
    ) -> Self {
        Self {
            _subcircuit: PhantomData,
            vk_file,
            left_proof,
            right_proof,
            prepare_agg_pk_chunks,
            commit_agg_pk_chunks,
            pks_commitment,
            prepare_signer_bitmap,
            prepare_agg_pk_commitment,
            commit_signer_bitmap,
            commit_agg_pk_commitment,
            position,
        }
    }
}

impl<SubCircuit: ConstraintSynthesizer<Fr>> ConstraintSynthesizer<MNT4Fr>
    for PKTreeNodeCircuit<SubCircuit>
{
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Load the verifying key from file.
        let mut file = File::open(format!("verifying_keys/{}", &self.vk_file))?;

        let vk_child = VerifyingKey::deserialize(&mut file).unwrap();

        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let sum_generator_g2_var =
            G2Gadget::alloc_constant(cs.ns(|| "alloc sum generator g2"), &sum_generator_g2_mnt6())?;

        let pedersen_generators_var = Vec::<G1Gadget>::alloc_constant(
            cs.ns(|| "alloc pedersen_generators"),
            pedersen_generators(5),
        )?;

        let vk_child_var = TheVkGadget::alloc_constant(cs.ns(|| "alloc vk child"), &vk_child)?;

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || { "Alloc private inputs" });

        let left_proof_var =
            TheProofGadget::alloc(cs.ns(|| "alloc left proof"), || Ok(&self.left_proof))?;

        let right_proof_var =
            TheProofGadget::alloc(cs.ns(|| "alloc right proof"), || Ok(&self.right_proof))?;

        let prepare_agg_pk_chunks_var =
            Vec::<G2Gadget>::alloc(cs.ns(|| "alloc prepare agg pk chunks"), || {
                Ok(&self.prepare_agg_pk_chunks[..])
            })?;

        let commit_agg_pk_chunks_var =
            Vec::<G2Gadget>::alloc(cs.ns(|| "alloc commit agg pk chunks"), || {
                Ok(&self.commit_agg_pk_chunks[..])
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

        // Calculating the prepare aggregate public key. All the chunks come with the generator added,
        // so we need to subtract it in order to get the correct aggregate public key. This is necessary
        // because we could have a chunk of public keys with no signers, thus resulting in it being
        // zero.
        next_cost_analysis!(cs, cost, || { "Calculate prepare agg pk" });

        let mut prepare_agg_pk = sum_generator_g2_var.clone();

        for i in 0..self.prepare_agg_pk_chunks.len() {
            prepare_agg_pk = prepare_agg_pk.add(
                cs.ns(|| format!("add next key, prepare {}", i)),
                &prepare_agg_pk_chunks_var[i],
            )?;

            prepare_agg_pk = prepare_agg_pk.sub(
                cs.ns(|| format!("subtract generator, prepare {}", i)),
                &sum_generator_g2_var,
            )?;
        }

        // Verifying prepare aggregate public key commitment. It just checks that the prepare
        // aggregate public key given as private input is correct by committing to it and comparing
        // the result with the prepare aggregate public key commitment given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify prepare agg pk commitment" });

        let prepare_agg_pk_bits =
            SerializeGadget::serialize_g2(cs.ns(|| "serialize prepare agg pk"), &prepare_agg_pk)?;

        let pedersen_hash = PedersenHashGadget::evaluate(
            cs.ns(|| "prepare agg pk pedersen hash"),
            &prepare_agg_pk_bits,
            &pedersen_generators_var,
        )?;

        let pedersen_bits = SerializeGadget::serialize_g1(
            cs.ns(|| "serialize prepare pedersen hash"),
            &pedersen_hash,
        )?;

        let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

        let mut reference_commitment = Vec::new();

        for i in 0..pedersen_bits.len() / 8 {
            reference_commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
        }

        prepare_agg_pk_commitment_var.enforce_equal(
            cs.ns(|| "prepare agg pk commitment == reference commitment"),
            &reference_commitment,
        )?;

        // Calculating the commitments to each of the prepare aggregate public keys chunks. These
        // will be given as input to the SNARK circuits lower on the tree.
        next_cost_analysis!(cs, cost, || {
            "Calculate prepare agg pk chunks commitments"
        });

        let mut prepare_agg_pk_chunks_commitments = Vec::new();

        for i in 0..prepare_agg_pk_chunks_var.len() {
            let chunk_bits = SerializeGadget::serialize_g2(
                cs.ns(|| format!("serialize prepare agg pk chunk {}", i)),
                &prepare_agg_pk_chunks_var[i],
            )?;

            let pedersen_hash = PedersenHashGadget::evaluate(
                cs.ns(|| format!("pedersen hash prepare agg pk chunk {}", i)),
                &chunk_bits,
                &pedersen_generators_var,
            )?;

            let pedersen_bits = SerializeGadget::serialize_g1(
                cs.ns(|| format!("serialize pedersen hash, prepare chunk {}", i)),
                &pedersen_hash,
            )?;

            let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

            let mut commitment = Vec::new();

            for i in 0..pedersen_bits.len() / 8 {
                commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
            }

            prepare_agg_pk_chunks_commitments.push(commitment);
        }

        // Calculating the commit aggregate public key. All the chunks come with the generator added,
        // so we need to subtract it in order to get the correct aggregate public key. This is necessary
        // because we could have a chunk of public keys with no signers, thus resulting in it being
        // zero.
        next_cost_analysis!(cs, cost, || { "Calculate commit agg pk" });

        let mut commit_agg_pk = sum_generator_g2_var.clone();

        for i in 0..self.commit_agg_pk_chunks.len() {
            commit_agg_pk = commit_agg_pk.add(
                cs.ns(|| format!("add next key, commit {}", i)),
                &commit_agg_pk_chunks_var[i],
            )?;

            commit_agg_pk = commit_agg_pk.sub(
                cs.ns(|| format!("subtract generator, commit {}", i)),
                &sum_generator_g2_var,
            )?;
        }

        // Verifying commit aggregate public key commitment. It just checks that the commit
        // aggregate public key given as private input is correct by committing to it and comparing
        // the result with the commit aggregate public key commitment given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify commit agg pk commitment" });

        let commit_agg_pk_bits =
            SerializeGadget::serialize_g2(cs.ns(|| "serialize commit agg pk"), &commit_agg_pk)?;

        let pedersen_hash = PedersenHashGadget::evaluate(
            cs.ns(|| "commit agg pk pedersen hash"),
            &commit_agg_pk_bits,
            &pedersen_generators_var,
        )?;

        let pedersen_bits = SerializeGadget::serialize_g1(
            cs.ns(|| "serialize commit pedersen hash"),
            &pedersen_hash,
        )?;

        let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

        let mut reference_commitment = Vec::new();

        for i in 0..pedersen_bits.len() / 8 {
            reference_commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
        }

        commit_agg_pk_commitment_var.enforce_equal(
            cs.ns(|| "commit agg pk commitment == reference commitment"),
            &reference_commitment,
        )?;

        // Calculating the commitments to each of the commit aggregate public keys chunks. These
        // will be given as input to the SNARK circuits lower on the tree.
        next_cost_analysis!(cs, cost, || {
            "Calculate commit agg pk chunks commitments"
        });

        let mut commit_agg_pk_chunks_commitments = Vec::new();

        for i in 0..commit_agg_pk_chunks_var.len() {
            let chunk_bits = SerializeGadget::serialize_g2(
                cs.ns(|| format!("serialize commit agg pk chunk {}", i)),
                &commit_agg_pk_chunks_var[i],
            )?;

            let pedersen_hash = PedersenHashGadget::evaluate(
                cs.ns(|| format!("pedersen hash commit agg pk chunk {}", i)),
                &chunk_bits,
                &pedersen_generators_var,
            )?;

            let pedersen_bits = SerializeGadget::serialize_g1(
                cs.ns(|| format!("serialize pedersen hash, commit chunk {}", i)),
                &pedersen_hash,
            )?;

            let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);

            let mut commitment = Vec::new();

            for i in 0..pedersen_bits.len() / 8 {
                commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
            }

            commit_agg_pk_chunks_commitments.push(commitment);
        }

        // Calculate the position for the left and right child nodes. Given the current position P,
        // the left position L and the right position R are given as:
        //    L = 2 * P
        //    R = 2 * P + 1
        // For efficiency reasons, we actually calculate the positions using bit manipulation.
        next_cost_analysis!(cs, cost, || { "Calculate positions" });

        // Get P.
        let mut bits = position_var.into_bits_le();

        // Calculate P << 1, which is equivalent to calculating 2 * P.
        bits.pop();
        bits.insert(0, Boolean::Constant(false));
        let left_position = UInt8::from_bits_le(&bits);

        // bits is currently P << 1 = L. Calculate L & 1, which is equivalent to L + 1.
        bits.remove(0);
        bits.insert(0, Boolean::Constant(true));
        let right_position = UInt8::from_bits_le(&bits);

        // Verify the ZK proof for the left child node.
        next_cost_analysis!(cs, cost, || { "Verify left ZK proof" });
        let mut proof_inputs = RecursiveInputGadget::to_field_elements::<Fr>(&pk_commitment_var)?;

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_agg_pk_chunks_commitments[0],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_agg_pk_chunks_commitments[1],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_agg_pk_chunks_commitments[0],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_agg_pk_chunks_commitments[1],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(&[
            left_position,
        ])?);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem<SubCircuit>, Fq>>::check_verify(
            cs.ns(|| "verify left groth16 proof"),
            &vk_child_var,
            proof_inputs.iter(),
            &left_proof_var,
        )?;

        // Verify the ZK proof for the right child node.
        next_cost_analysis!(cs, cost, || { "Verify right ZK proof" });
        let mut proof_inputs = RecursiveInputGadget::to_field_elements::<Fr>(&pk_commitment_var)?;

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_agg_pk_chunks_commitments[2],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_agg_pk_chunks_commitments[3],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_agg_pk_chunks_commitments[2],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_agg_pk_chunks_commitments[3],
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(&[
            right_position,
        ])?);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem<SubCircuit>, Fq>>::check_verify(
            cs.ns(|| "verify right groth16 proof"),
            &vk_child_var,
            proof_inputs.iter(),
            &right_proof_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
