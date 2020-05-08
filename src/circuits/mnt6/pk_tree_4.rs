use std::fs::File;

use algebra::mnt4_753::{Fq, Fr, MNT4_753};
use algebra::mnt6_753::Fr as MNT6Fr;
use algebra_core::FromBytes;
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::groth16::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt4_753::PairingGadget;
use r1cs_std::prelude::*;

use crate::circuits::mnt4::PKTree5Circuit;
use crate::gadgets::input::RecursiveInputGadget;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the wrapper circuit just by editing these types.
type TheProofSystem = Groth16<MNT4_753, PKTree5Circuit, Fr>;
type TheProofGadget = ProofGadget<MNT4_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT4_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT4_753, Fq, PairingGadget>;

/// This is the fourth level of the PKTreeCircuit. See PKTree0Circuit for more details.
#[derive(Clone)]
pub struct PKTree4Circuit {
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

impl PKTree4Circuit {
    pub fn new(
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

impl ConstraintSynthesizer<MNT6Fr> for PKTree4Circuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Load the verifying key from file.
        let mut file = File::open("verifying_keys/pk_tree_5.bin")?;

        let alpha_g1 = FromBytes::read(&mut file)?;

        let beta_g2 = FromBytes::read(&mut file)?;

        let gamma_g2 = FromBytes::read(&mut file)?;

        let delta_g2 = FromBytes::read(&mut file)?;

        let gamma_abc_g1_len: u64 = FromBytes::read(&mut file)?;

        let mut gamma_abc_g1 = vec![];

        for _ in 0..gamma_abc_g1_len {
            gamma_abc_g1.push(FromBytes::read(&mut file)?);
        }

        let vk_child = VerifyingKey {
            alpha_g1,
            beta_g2,
            gamma_g2,
            delta_g2,
            gamma_abc_g1,
        };

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

        let position_var = UInt8::alloc_input(cs.ns(|| "alloc position"), || Ok(self.position))?;

        // Calculate the position for the left and right child nodes. Given the current position P,
        // the left position L and the right position R are given as:
        //    L = 2 * P
        //    R = 2 * P + 1
        // For efficiency reasons, we calculate the positions using bit manipulation.
        next_cost_analysis!(cs, cost, || { "Calculate positions" });

        let mut bits = position_var.into_bits_le();

        bits.pop();
        bits.insert(0, Boolean::Constant(false));
        let left_position = vec![UInt8::from_bits_le(&bits)];

        bits.remove(0);
        bits.insert(0, Boolean::Constant(true));
        let right_position = vec![UInt8::from_bits_le(&bits)];

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

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &left_position,
        )?);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem, Fq>>::check_verify(
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

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &right_position,
        )?);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify right groth16 proof"),
            &vk_child_var,
            proof_inputs.iter(),
            &right_proof_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
