use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fq, Fr, G2Projective, MNT6_753};
use algebra_core::ProjectiveCurve;
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::{FqGadget, G1Gadget, G2Gadget, PairingGadget};
use r1cs_std::prelude::*;

use crate::circuits::mnt6::PKTree0Circuit;
use crate::constants::{
    sum_generator_g1_mnt6, sum_generator_g2_mnt6, EPOCH_LENGTH, MAX_NON_SIGNERS, VALIDATOR_SLOTS,
};
use crate::gadgets::input::RecursiveInputGadget;
use crate::gadgets::mnt4::{
    MacroBlockGadget, MerkleTreeGadget, SerializeGadget, StateCommitmentGadget,
};
use crate::primitives::{pedersen_generators, MacroBlock};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the wrapper circuit just by editing these types.
type TheProofSystem = Groth16<MNT6_753, PKTree0Circuit, Fr>;
type TheProofGadget = ProofGadget<MNT6_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT6_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT6_753, Fq, PairingGadget>;

/// This is the macro block circuit. It takes as inputs an initial state commitment and final state commitment
/// and it produces a proof that there exists a valid macro block that transforms the initial state
/// into the final state.
/// Since the state is composed only of the block number and the public keys of the current validator
/// list, updating the state is just incrementing the block number and substituting the previous
/// public keys with the public keys of the new validator list.
#[derive(Clone)]
pub struct MacroBlockCircuit {
    // Private inputs
    prepare_agg_pk_chunks: Vec<G2Projective>,
    commit_agg_pk_chunks: Vec<G2Projective>,
    initial_pks_commitment: Vec<u8>,
    initial_block_number: u32,
    final_pks_commitment: Vec<u8>,
    block: MacroBlock,
    proof: Proof<MNT6_753>,
    vk: VerifyingKey<MNT6_753>,

    // Public inputs
    initial_state_commitment: Vec<u8>,
    final_state_commitment: Vec<u8>,
}

impl MacroBlockCircuit {
    pub fn new(
        prepare_agg_pk_chunks: Vec<G2Projective>,
        commit_agg_pk_chunks: Vec<G2Projective>,
        initial_pks_commitment: Vec<u8>,
        initial_block_number: u32,
        final_pks_commitment: Vec<u8>,
        block: MacroBlock,
        proof: Proof<MNT6_753>,
        vk: VerifyingKey<MNT6_753>,
        initial_state_commitment: Vec<u8>,
        final_state_commitment: Vec<u8>,
    ) -> Self {
        Self {
            prepare_agg_pk_chunks,
            commit_agg_pk_chunks,
            initial_pks_commitment,
            initial_block_number,
            final_pks_commitment,
            block,
            proof,
            vk,
            initial_state_commitment,
            final_state_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for MacroBlockCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let epoch_length_var = UInt32::constant(EPOCH_LENGTH);

        let max_non_signers_var = FqGadget::alloc_constant(
            cs.ns(|| "alloc max non signers"),
            &Fq::from(MAX_NON_SIGNERS as u64),
        )?;

        let sig_generator_var = G2Gadget::alloc_constant(
            cs.ns(|| "alloc signature generator"),
            &G2Projective::prime_subgroup_generator(),
        )?;

        let sum_generator_g1_var =
            G1Gadget::alloc_constant(cs.ns(|| "alloc sum generator g1"), &sum_generator_g1_mnt6())?;

        let sum_generator_g2_var =
            G2Gadget::alloc_constant(cs.ns(|| "alloc sum generator g2"), &sum_generator_g2_mnt6())?;

        let pedersen_generators_var = Vec::<G1Gadget>::alloc_constant(
            cs.ns(|| "alloc pedersen_generators"),
            pedersen_generators(256),
        )?;

        // TODO: This needs to be changed to a constant!
        let vk_var = TheVkGadget::alloc(cs.ns(|| "alloc vk"), || Ok(&self.vk))?;

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || "Alloc private inputs");

        let prepare_agg_pk_chunks_var =
            Vec::<G2Gadget>::alloc(cs.ns(|| "alloc prepare agg pk chunks"), || {
                Ok(&self.prepare_agg_pk_chunks[..])
            })?;

        let commit_agg_pk_chunks_var =
            Vec::<G2Gadget>::alloc(cs.ns(|| "alloc commit agg pk chunks"), || {
                Ok(&self.commit_agg_pk_chunks[..])
            })?;

        let initial_pks_commitment_var = UInt8::alloc_vec(
            cs.ns(|| "alloc initial pks commitment"),
            self.initial_pks_commitment.as_ref(),
        )?;

        let initial_block_number_var = UInt32::alloc(
            cs.ns(|| "alloc initial block number"),
            Some(self.initial_block_number),
        )?;

        let final_pks_commitment_var = UInt8::alloc_vec(
            cs.ns(|| "alloc final pks commitment"),
            self.final_pks_commitment.as_ref(),
        )?;

        let block_var = MacroBlockGadget::alloc(cs.ns(|| "alloc macro block"), || Ok(&self.block))?;

        let proof_var = TheProofGadget::alloc(cs.ns(|| "alloc proof"), || Ok(&self.proof))?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let initial_state_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc initial state commitment"),
            self.initial_state_commitment.as_ref(),
        )?;

        let final_state_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "alloc final state commitment"),
            self.final_state_commitment.as_ref(),
        )?;

        // Verifying equality for initial state commitment. It just checks that the initial block
        // number and the initial public keys commitment given as private inputs are correct by
        // committing to them and comparing the result with the initial state commitment given as a
        // public input.
        next_cost_analysis!(cs, cost, || { "Verify initial state commitment" });

        let reference_commitment = StateCommitmentGadget::evaluate(
            cs.ns(|| "reference initial state commitment"),
            &initial_block_number_var,
            &initial_pks_commitment_var,
            &pedersen_generators_var,
            &sum_generator_g1_var,
        )?;

        initial_state_commitment_var.enforce_equal(
            cs.ns(|| "initial state commitment == reference commitment"),
            &reference_commitment,
        )?;

        // Verifying equality for final state commitment. It just checks that the calculated final
        // block number and the final public keys commitment given as private input are correct by
        // committing to them and comparing the result with the final state commitment given as a
        // public input.
        next_cost_analysis!(cs, cost, || { "Verify final state commitment" });

        let final_block_number_var = UInt32::addmany(
            cs.ns(|| format!("increment block number")),
            &[initial_block_number_var.clone(), epoch_length_var.clone()],
        )?;

        let reference_commitment = StateCommitmentGadget::evaluate(
            cs.ns(|| "reference final state commitment"),
            &final_block_number_var,
            &final_pks_commitment_var,
            &pedersen_generators_var,
            &sum_generator_g1_var,
        )?;

        final_state_commitment_var.enforce_equal(
            cs.ns(|| "final state commitment == reference commitment"),
            &reference_commitment,
        )?;

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

        prepare_agg_pk = prepare_agg_pk.sub(
            cs.ns(|| "subtract generator, prepare"),
            &sum_generator_g2_var,
        )?;

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

        commit_agg_pk = commit_agg_pk.sub(
            cs.ns(|| "subtract generator, commit"),
            &sum_generator_g2_var,
        )?;

        // Calculating the Merkle tree over the prepare aggregate public key chunks.
        next_cost_analysis!(cs, cost, || { "Calculate prepare agg pk commitment" });

        let mut bits = Vec::new();

        for i in 0..self.prepare_agg_pk_chunks.len() {
            bits.push(SerializeGadget::serialize_g2(
                cs.ns(|| format!("serialize prepare agg pk: chunk {}", i)),
                &prepare_agg_pk_chunks_var[i],
            )?);
        }

        let prepare_agg_pk_commitment = MerkleTreeGadget::construct(
            cs.ns(|| "calculate prepare agg pk commitment"),
            &bits,
            &pedersen_generators_var,
            &sum_generator_g1_var,
        )?;

        // Calculating the Merkle tree over the commit aggregate public key chunks.
        next_cost_analysis!(cs, cost, || { "Calculate commit agg pk commitment" });

        let mut bits = Vec::new();

        for i in 0..self.commit_agg_pk_chunks.len() {
            bits.push(SerializeGadget::serialize_g2(
                cs.ns(|| format!("serialize commit agg pk: chunk {}", i)),
                &commit_agg_pk_chunks_var[i],
            )?);
        }

        let commit_agg_pk_commitment = MerkleTreeGadget::construct(
            cs.ns(|| "calculate commit agg pk commitment"),
            &bits,
            &pedersen_generators_var,
            &sum_generator_g1_var,
        )?;

        // Preparing the inputs for the SNARK proof verification. All the inputs need to be Vec<UInt8>,
        // so they need to be converted to such.
        next_cost_analysis!(cs, cost, || { "Prepare inputs for SNARK" });

        let position = vec![UInt8::constant(0)];

        let mut prepare_signer_bitmap_bytes = Vec::new();

        for i in 0..VALIDATOR_SLOTS / 8 {
            prepare_signer_bitmap_bytes.push(UInt8::from_bits_le(
                &block_var.prepare_signer_bitmap[i * 8..(i + 1) * 8],
            ));
        }

        let mut commit_signer_bitmap_bytes = Vec::new();

        for i in 0..VALIDATOR_SLOTS / 8 {
            commit_signer_bitmap_bytes.push(UInt8::from_bits_le(
                &block_var.prepare_signer_bitmap[i * 8..(i + 1) * 8],
            ));
        }

        // Verifying the SNARK proof. This is a proof that the aggregate public keys chunks are indeed
        // correct. It simply takes the public keys and the signer bitmaps and recalculates the aggregate
        // public keys chunks, then compares them to the aggregate public keys chunks given as public input.
        // Internally, this SNARK circuit is very complex. See the PKTree0Circuit for more details.
        next_cost_analysis!(cs, cost, || { "Verify SNARK proof" });

        let mut proof_inputs =
            RecursiveInputGadget::to_field_elements::<Fr>(&initial_pks_commitment_var)?;

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_signer_bitmap_bytes,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &prepare_agg_pk_commitment,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_signer_bitmap_bytes,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &commit_agg_pk_commitment,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &position,
        )?);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify groth16 proof"),
            &vk_var,
            proof_inputs.iter(),
            &proof_var,
        )?;

        // Verifying that the block is valid. Note that we give it the initial block number and the
        // final public keys commitment. That's because the "state" is composed of the public keys of
        // the current validator list and the block number of the next macro block. So, a macro block
        // actually has the number from the previous state and contains the public keys of the next
        // state.
        next_cost_analysis!(cs, cost, || "Verify block");

        block_var.verify(
            cs.ns(|| "verify block"),
            &final_pks_commitment_var,
            &initial_block_number_var,
            &prepare_agg_pk,
            &commit_agg_pk,
            &max_non_signers_var,
            &sig_generator_var,
            &sum_generator_g1_var,
            &pedersen_generators_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
