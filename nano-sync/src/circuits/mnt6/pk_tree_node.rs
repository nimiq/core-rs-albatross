use ark_crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use ark_crypto_primitives::NIZKVerifierGadget;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt4_753::{Fq, Fr, MNT4_753};
use ark_mnt6_753::Fr as MNT6Fr;
use ark_r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use ark_r1cs_std::mnt4_753::PairingGadget;
use ark_r1cs_std::prelude::*;
use ark_serialize::CanonicalDeserialize;
use std::fs::File;
use std::marker::PhantomData;

use crate::gadgets::input::RecursiveInputGadget;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience.
type TheProofSystem<T> = Groth16<MNT4_753, T, Fr>;
type TheProofGadget = ProofGadget<MNT4_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT4_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT4_753, Fq, PairingGadget>;

/// This is the node subcircuit of the PKTreeCircuit. See PKTreeLeafCircuit for more details.
/// It is different from the other node subcircuit on the MNT4 curve in that it doesn't recalculate
/// the aggregate public key commitments, it just passes them on to the next level.
#[derive(Clone)]
pub struct PKTreeNodeCircuit<SubCircuit> {
    _subcircuit: PhantomData<SubCircuit>,

    // Path to the verifying key file. Not an input to the SNARK circuit.
    vk_file: &'static str,

    // Private inputs
    left_proof: Proof<MNT4_753>,
    right_proof: Proof<MNT4_753>,

    // Public inputs
    pks_commitment: Vec<u8>,
    prepare_signer_bitmap: Vec<u8>,
    left_prepare_agg_pk_commitment: Vec<u8>,
    right_prepare_agg_pk_commitment: Vec<u8>,
    commit_signer_bitmap: Vec<u8>,
    left_commit_agg_pk_commitment: Vec<u8>,
    right_commit_agg_pk_commitment: Vec<u8>,
    position: u8,
}

impl<SubCircuit> PKTreeNodeCircuit<SubCircuit> {
    pub fn new(
        vk_file: &'static str,
        left_proof: Proof<MNT4_753>,
        right_proof: Proof<MNT4_753>,
        pks_commitment: Vec<u8>,
        prepare_signer_bitmap: Vec<u8>,
        left_prepare_agg_pk_commitment: Vec<u8>,
        right_prepare_agg_pk_commitment: Vec<u8>,
        commit_signer_bitmap: Vec<u8>,
        left_commit_agg_pk_commitment: Vec<u8>,
        right_commit_agg_pk_commitment: Vec<u8>,
        position: u8,
    ) -> Self {
        Self {
            _subcircuit: PhantomData,
            vk_file,
            left_proof,
            right_proof,
            pks_commitment,
            prepare_signer_bitmap,
            left_prepare_agg_pk_commitment,
            right_prepare_agg_pk_commitment,
            commit_signer_bitmap,
            left_commit_agg_pk_commitment,
            right_commit_agg_pk_commitment,
            position,
        }
    }
}

impl<SubCircuit: ConstraintSynthesizer<Fr>> ConstraintSynthesizer<MNT6Fr>
    for PKTreeNodeCircuit<SubCircuit>
{
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Load the verifying key from file.
        let mut file = File::open(format!("verifying_keys/{}", &self.vk_file))?;

        let vk_child = VerifyingKey::deserialize(&mut file).unwrap();

        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let vk_child_var = TheVkGadget::alloc_constant(cs.ns(|| "alloc vk child"), &vk_child)?;

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || { "Alloc private inputs" });

        let left_proof_var =
            TheProofGadget::alloc(cs.ns(|| "alloc left proof"), || Ok(&self.left_proof))?;

        let right_proof_var =
            TheProofGadget::alloc(cs.ns(|| "alloc right proof"), || Ok(&self.right_proof))?;

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

        let left_prepare_agg_pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc left prepare aggregate pk commitment"),
            self.left_prepare_agg_pk_commitment.as_ref(),
        )?;

        let right_prepare_agg_pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc right prepare aggregate pk commitment"),
            self.right_prepare_agg_pk_commitment.as_ref(),
        )?;

        let commit_signer_bitmap_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc commit signer bitmap"),
            self.commit_signer_bitmap.as_ref(),
        )?;

        let left_commit_agg_pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc left commit aggregate pk commitment"),
            self.left_commit_agg_pk_commitment.as_ref(),
        )?;

        let right_commit_agg_pk_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc right commit aggregate pk commitment"),
            self.right_commit_agg_pk_commitment.as_ref(),
        )?;

        let position_var = UInt8::alloc_input_vec(cs.ns(|| "alloc position"), &[self.position])?
            .pop()
            .unwrap();

        // Calculate the position for the left and right child nodes. Given the current position P,
        // the left position L and the right position R are given as:
        //    L = 2 * P
        //    R = 2 * P + 1
        // For efficiency reasons, we calculate the positions using bit manipulation.
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
            &left_prepare_agg_pk_commitment_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &left_commit_agg_pk_commitment_var,
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
            &right_prepare_agg_pk_commitment_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &right_commit_agg_pk_commitment_var,
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
