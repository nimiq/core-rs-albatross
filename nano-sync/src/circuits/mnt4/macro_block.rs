use ark_crypto_primitives::snark::BooleanInputVar;
use ark_crypto_primitives::SNARKGadget;
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::{Proof, VerifyingKey};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var, PairingVar};
use ark_mnt6_753::{Fq, G2Projective, MNT6_753};
use ark_r1cs_std::prelude::{
    AllocVar, Boolean, CurveVar, EqGadget, FieldVar, ToBitsGadget, UInt32,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::constants::EPOCH_LENGTH;
use crate::gadgets::mnt4::{
    MacroBlockGadget, PedersenHashGadget, SerializeGadget, StateCommitmentGadget,
};
use crate::primitives::{pedersen_generators, MacroBlock};
use crate::utils::{pack_inputs, unpack_inputs};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is the macro block circuit. It takes as inputs an initial state commitment and final state commitment
/// and it produces a proof that there exists a valid macro block that transforms the initial state
/// into the final state.
/// Since the state is composed only of the block number and the public keys of the current validator
/// list, updating the state is just incrementing the block number and substituting the previous
/// public keys with the public keys of the new validator list.
#[derive(Clone)]
pub struct MacroBlockCircuit {
    // Verifying key for the PKTree circuit. Not an input to the SNARK circuit.
    vk_pk_tree: VerifyingKey<MNT6_753>,

    // Witnesses (private)
    agg_pk_chunks: Vec<G2Projective>,
    proof: Proof<MNT6_753>,
    initial_pk_tree_root: Vec<bool>,
    initial_block_number: u32,
    initial_round_number: u32,
    final_pk_tree_root: Vec<bool>,
    block: MacroBlock,

    // Inputs (public)
    // Our inputs are always vectors of booleans (semantically), so that they are consistent across
    // the different elliptic curves that we use. However, for compactness, we represent them as
    // field elements. Both of the curves that we use have a modulus of 753 bits and a capacity
    // of 752 bits. So, the first 752 bits (in little-endian) of each field element is data, and the
    // last bit is always set to zero.
    initial_state_commitment: Vec<Fq>,
    final_state_commitment: Vec<Fq>,
}

impl MacroBlockCircuit {
    pub fn new(
        vk_pk_tree: VerifyingKey<MNT6_753>,
        agg_pk_chunks: Vec<G2Projective>,
        proof: Proof<MNT6_753>,
        initial_pk_tree_root: Vec<bool>,
        initial_block_number: u32,
        initial_round_number: u32,
        final_pk_tree_root: Vec<bool>,
        block: MacroBlock,
        initial_state_commitment: Vec<Fq>,
        final_state_commitment: Vec<Fq>,
    ) -> Self {
        Self {
            vk_pk_tree,
            agg_pk_chunks,
            proof,
            initial_pk_tree_root,
            initial_block_number,
            initial_round_number,
            final_pk_tree_root,
            block,
            initial_state_commitment,
            final_state_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for MacroBlockCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let epoch_length_var = UInt32::<MNT4Fr>::new_constant(cs.clone(), EPOCH_LENGTH)?;

        let pedersen_generators_var =
            Vec::<G1Var>::new_constant(cs.clone(), pedersen_generators(5))?;

        let vk_pk_tree_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(cs.clone(), &self.vk_pk_tree)?;

        // Allocate all the witnesses.
        next_cost_analysis!(cs, cost, || { "Alloc witnesses" });

        let agg_pk_chunks_var =
            Vec::<G2Var>::new_witness(cs.clone(), || Ok(&self.agg_pk_chunks[..]))?;

        let proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        let initial_pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.initial_pk_tree_root[..]))?;

        let initial_block_number_var =
            UInt32::new_witness(cs.clone(), || Ok(&self.initial_block_number))?;

        let initial_round_number_var =
            UInt32::new_witness(cs.clone(), || Ok(&self.initial_round_number))?;

        let final_pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.final_pk_tree_root[..]))?;

        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(&self.block))?;

        // Allocate all the inputs.
        next_cost_analysis!(cs, cost, || { "Alloc inputs" });

        let initial_state_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.initial_state_commitment[..]))?;

        let final_state_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.final_state_commitment[..]))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        next_cost_analysis!(cs, cost, || { "Unpack inputs" });

        let initial_state_commitment_bits =
            unpack_inputs(initial_state_commitment_var)?[..760].to_vec();

        let final_state_commitment_bits =
            unpack_inputs(final_state_commitment_var)?[..760].to_vec();

        // Verifying equality for initial state commitment. It just checks that the initial block
        // number and the initial public key tree root given as witnesses are correct by committing
        // to them and comparing the result with the initial state commitment given as an input.
        next_cost_analysis!(cs, cost, || { "Verify initial state commitment" });

        let reference_commitment = StateCommitmentGadget::evaluate(
            cs.clone(),
            &initial_block_number_var,
            &initial_pk_tree_root_var,
            &pedersen_generators_var,
        )?;

        initial_state_commitment_bits.enforce_equal(&reference_commitment)?;

        // Verifying equality for final state commitment. It just checks that the calculated final
        // block number and the final public key tree root given as a witness are correct by committing
        // to them and comparing the result with the final state commitment given as an input.
        next_cost_analysis!(cs, cost, || { "Verify final state commitment" });

        let final_block_number_var =
            UInt32::addmany(&[initial_block_number_var.clone(), epoch_length_var])?;

        let reference_commitment = StateCommitmentGadget::evaluate(
            cs.clone(),
            &final_block_number_var,
            &final_pk_tree_root_var,
            &pedersen_generators_var,
        )?;

        final_state_commitment_bits.enforce_equal(&reference_commitment)?;

        // Calculating the commitments to each of the aggregate public keys chunks. These will be
        // given as input to the PKTree SNARK circuit.
        next_cost_analysis!(cs, cost, || { "Calculate agg pk chunks commitments" });

        let mut agg_pk_chunks_commitments = Vec::new();

        for chunk in &agg_pk_chunks_var {
            let chunk_bits = SerializeGadget::serialize_g2(cs.clone(), chunk)?;

            let pedersen_hash =
                PedersenHashGadget::evaluate(&chunk_bits, &pedersen_generators_var)?;

            let pedersen_bits = SerializeGadget::serialize_g1(cs.clone(), &pedersen_hash)?;

            agg_pk_chunks_commitments.push(pedersen_bits);
        }

        // Verifying the SNARK proof. This is a proof that the aggregate public key is indeed
        // correct. It simply takes the public keys and the signers bitmap and recalculates the
        // aggregate public key and then compares it to the aggregate public key given as public
        // input.
        // Internally, this SNARK circuit is very complex. See the PKTreeLeaf circuit for more details.
        // Note that in this particular case, we don't pass the aggregated public key to the SNARK.
        // Instead we pass two chunks of the aggregated public key to it. This is just because the
        // PKTreeNode circuit in the MNT6 curve takes two chunks as inputs.
        next_cost_analysis!(cs, cost, || { "Verify SNARK proof" });

        let mut proof_inputs = pack_inputs(initial_pk_tree_root_var);

        proof_inputs.append(&mut pack_inputs(agg_pk_chunks_commitments[0].to_bits_le()?));

        proof_inputs.append(&mut pack_inputs(agg_pk_chunks_commitments[1].to_bits_le()?));

        proof_inputs.append(&mut pack_inputs(block_var.signer_bitmap.clone()));

        // Since we are beginning at the root of the PKTree our path is all zeros.
        proof_inputs.append(&mut pack_inputs(FqVar::zero().to_bits_le()?));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_pk_tree_var,
            &input_var,
            &proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Calculating the aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate agg pk" });

        let mut agg_pk_var = G2Var::zero();

        for pk in &agg_pk_chunks_var {
            agg_pk_var += pk;
        }

        // Verifying that the block is valid. Note that we give it the initial block number and the
        // final public keys commitment. That's because the "state" is composed of the public keys of
        // the current validator list and the block number of the next macro block. So, a macro block
        // actually has the number from the previous state and contains the public keys of the next
        // state.
        next_cost_analysis!(cs, cost, || "Verify block");

        block_var
            .verify(
                cs.clone(),
                &final_pk_tree_root_var,
                &initial_block_number_var,
                &initial_round_number_var,
                &agg_pk_var,
                &pedersen_generators_var,
            )?
            .enforce_equal(&Boolean::constant(true))?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
