use algebra::mnt4_753::{Fq, Fr, MNT4_753};
use algebra::mnt6_753::Fr as MNT6Fr;
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::groth16::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt4_753::PairingGadget;
use r1cs_std::prelude::*;

use crate::circuits::mnt4::PKTree3Circuit;
use crate::gadgets::input::RecursiveInputGadget;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the wrapper circuit just by editing these types.
type TheProofSystem = Groth16<MNT4_753, PKTree3Circuit, Fr>;
type TheProofGadget = ProofGadget<MNT4_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT4_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT4_753, Fq, PairingGadget>;

/// This is the second level of the PKTreeCircuit. See PKTree0Circuit for more details.
#[derive(Clone)]
pub struct PKTree2Circuit {
    // Private inputs
    left_proof: Proof<MNT4_753>,
    right_proof: Proof<MNT4_753>,
    vk_child: VerifyingKey<MNT4_753>,

    // Public inputs
    pk_commitment: Vec<u8>,
    prepare_signer_bitmap: Vec<u8>,
    prepare_agg_pk_commitment: Vec<u8>,
    commit_signer_bitmap: Vec<u8>,
    commit_agg_pk_commitment: Vec<u8>,
    position: u8,
}

impl PKTree2Circuit {
    pub fn new(
        left_proof: Proof<MNT4_753>,
        right_proof: Proof<MNT4_753>,
        vk_child: VerifyingKey<MNT4_753>,
        pk_commitment: Vec<u8>,
        prepare_signer_bitmap: Vec<u8>,
        prepare_agg_pk_commitment: Vec<u8>,
        commit_signer_bitmap: Vec<u8>,
        commit_agg_pk_commitment: Vec<u8>,
        position: u8,
    ) -> Self {
        Self {
            left_proof,
            right_proof,
            vk_child,
            pk_commitment,
            prepare_signer_bitmap,
            prepare_agg_pk_commitment,
            commit_signer_bitmap,
            commit_agg_pk_commitment,
            position,
        }
    }
}

impl ConstraintSynthesizer<MNT6Fr> for PKTree2Circuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        // TODO: This needs to be changed to a constant!
        let vk_child_var = TheVkGadget::alloc(cs.ns(|| "alloc vk child"), || Ok(&self.vk_child))?;

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

        // Calculate the position for the left and right child nodes. Given the current position P,
        // the left position L and the right position R are given as:
        //    L = 2 * P
        //    R = 2 * P + 1
        // For efficiency reasons, we actually calculate the positions using bit manipulation.
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
            &prepare_agg_pk_commitment_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_agg_pk_commitment_var,
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
            &prepare_agg_pk_commitment_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_signer_bitmap_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_agg_pk_commitment_var,
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
