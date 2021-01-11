use ark_ec::ProjectiveCurve;
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var, PairingVar};
use ark_mnt6_753::{Fq, Fr, G2Projective, MNT6_753};
use ark_r1cs_std::prelude::{AllocVar, CondSelectGadget, CurveVar, EqGadget, UInt32};
use ark_relations::r1cs::{
    ConstraintSynthesizer, ConstraintSystem, ConstraintSystemRef, SynthesisError,
};
use ark_serialize::CanonicalDeserialize;
use std::fs::File;

use crate::constants::{EPOCH_LENGTH, MIN_SIGNERS, VALIDATOR_SLOTS};
use crate::gadgets::mnt4::{
    MacroBlockGadget, PedersenHashGadget, SerializeGadget, StateCommitmentGadget,
};
use crate::primitives::{pedersen_generators, MacroBlock};
use crate::utils::{reverse_inner_byte_order, unpack_inputs};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};
use ark_r1cs_std::bits::boolean::Boolean;

/// This is the macro block circuit. It takes as inputs an initial state commitment and final state commitment
/// and it produces a proof that there exists a valid macro block that transforms the initial state
/// into the final state.
/// Since the state is composed only of the block number and the public keys of the current validator
/// list, updating the state is just incrementing the block number and substituting the previous
/// public keys with the public keys of the new validator list.
#[derive(Clone)]
pub struct MacroBlockCircuit {
    // Path to the verifying key file. Not an input to the SNARK circuit.
    vk_file: &'static str,

    // Witnesses (private)
    agg_pk_chunks: Vec<G2Projective>,
    proof: Proof<MNT6_753>,
    initial_pk_tree_root: Vec<bool>,
    initial_block_number: u32,
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
        vk_file: &'static str,
        agg_pk_chunks: Vec<G2Projective>,
        proof: Proof<MNT6_753>,
        initial_pk_tree_root: Vec<bool>,
        initial_block_number: u32,
        final_pk_tree_root: Vec<bool>,
        block: MacroBlock,
        initial_state_commitment: Vec<Fq>,
        final_state_commitment: Vec<Fq>,
    ) -> Self {
        Self {
            vk_file,
            agg_pk_chunks,
            proof,
            initial_pk_tree_root,
            initial_block_number,
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
        // Load the verifying key from file.
        let mut file = File::open(format!("verifying_keys/{}", &self.vk_file)).unwrap();

        let vk_pk_tree = VerifyingKey::deserialize(&mut file).unwrap();

        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let epoch_length_var = UInt32::<MNT4Fr>::constant(EPOCH_LENGTH);

        let pedersen_generators_var =
            Vec::<G1Var>::new_constant(cs.clone(), pedersen_generators(5))?;

        let vk_pk_tree_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(cs.clone(), vk_pk_tree)?;

        // Allocate all the witnesses.
        next_cost_analysis!(cs, cost, || { "Alloc witnesses" });

        let agg_pk_chunks_var =
            Vec::<G2Var>::new_witness(cs.clone(), || Ok(&self.agg_pk_chunks[..]))?;

        let proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.proof))?;

        let mut initial_pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&self.initial_pk_tree_root[..]))?;

        let initial_block_number_var =
            UInt32::new_witness(cs.clone(), || Ok(&self.initial_block_number))?;

        let mut final_pk_tree_root_var =
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
            &mut initial_pk_tree_root_var,
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
            &mut final_pk_tree_root_var,
            &pedersen_generators_var,
        )?;

        final_state_commitment_bits.enforce_equal(&reference_commitment)?;

        // // Calculating the commit aggregate public key. All the chunks come with the generator added,
        // // so we need to subtract it in order to get the correct aggregate public key. This is necessary
        // // because we could have a chunk of public keys with no signers, thus resulting in it being
        // // zero.
        // next_cost_analysis!(cs, cost, || { "Calculate commit agg pk" });
        //
        // let mut commit_agg_pk = sum_generator_g2_var.clone();
        //
        // for i in 0..self.commit_agg_pk_chunks.len() {
        //     commit_agg_pk = commit_agg_pk.add(
        //         cs.ns(|| format!("add next key, commit {}", i)),
        //         &commit_agg_pk_chunks_var[i],
        //     )?;
        //
        //     commit_agg_pk = commit_agg_pk.sub(
        //         cs.ns(|| format!("subtract generator, commit {}", i)),
        //         &sum_generator_g2_var,
        //     )?;
        // }
        //
        // commit_agg_pk = commit_agg_pk.sub(
        //     cs.ns(|| "subtract generator, commit"),
        //     &sum_generator_g2_var,
        // )?;
        //
        // // Calculating the commitments to each of the commit aggregate public keys chunks. These
        // // will be given as input to the PKTree SNARK circuit.
        // next_cost_analysis!(cs, cost, || {
        //     "Calculate commit agg pk chunks commitments"
        // });
        //
        // let mut commit_agg_pk_chunks_commitments = Vec::new();
        //
        // for i in 0..commit_agg_pk_chunks_var.len() {
        //     let chunk_bits = SerializeGadget::serialize_g2(
        //         cs.ns(|| format!("serialize commit agg pk chunk {}", i)),
        //         &commit_agg_pk_chunks_var[i],
        //     )?;
        //
        //     let pedersen_hash = PedersenHashGadget::evaluate(
        //         cs.ns(|| format!("pedersen hash commit agg pk chunk {}", i)),
        //         &chunk_bits,
        //         &pedersen_generators_var,
        //     )?;
        //
        //     let pedersen_bits = SerializeGadget::serialize_g1(
        //         cs.ns(|| format!("serialize pedersen hash, commit chunk {}", i)),
        //         &pedersen_hash,
        //     )?;
        //
        //     let pedersen_bits = reverse_inner_byte_order(&pedersen_bits[..]);
        //
        //     let mut commitment = Vec::new();
        //
        //     for i in 0..pedersen_bits.len() / 8 {
        //         commitment.push(UInt8::from_bits_le(&pedersen_bits[i * 8..(i + 1) * 8]));
        //     }
        //
        //     commit_agg_pk_chunks_commitments.push(commitment);
        // }
        //
        // // Preparing the inputs for the SNARK proof verification. All the inputs need to be Vec<UInt8>,
        // // so they need to be converted to such.
        // next_cost_analysis!(cs, cost, || { "Prepare inputs for SNARK" });
        //
        // let position = vec![UInt8::constant(0)];
        //
        // let mut prepare_signer_bitmap_bytes = Vec::new();
        //
        // for i in 0..VALIDATOR_SLOTS / 8 {
        //     prepare_signer_bitmap_bytes.push(UInt8::from_bits_le(
        //         &block_var.prepare_signer_bitmap[i * 8..(i + 1) * 8],
        //     ));
        // }
        //
        // let mut commit_signer_bitmap_bytes = Vec::new();
        //
        // for i in 0..VALIDATOR_SLOTS / 8 {
        //     commit_signer_bitmap_bytes.push(UInt8::from_bits_le(
        //         &block_var.prepare_signer_bitmap[i * 8..(i + 1) * 8],
        //     ));
        // }
        //
        // // Verifying the SNARK proof. This is a proof that the aggregate public keys chunks are indeed
        // // correct. It simply takes the public keys and the signer bitmaps and recalculates the aggregate
        // // public keys chunks, then compares them to the aggregate public keys chunks given as public input.
        // // Internally, this SNARK circuit is very complex. See the PKTreeLeafCircuit for more details.
        // next_cost_analysis!(cs, cost, || { "Verify SNARK proof" });
        //
        // let mut proof_inputs =
        //     RecursiveInputGadget::to_field_elements::<Fr>(&initial_pk_tree_root_var)?;
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &prepare_signer_bitmap_bytes,
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &prepare_agg_pk_chunks_commitments[0],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &prepare_agg_pk_chunks_commitments[1],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &commit_signer_bitmap_bytes,
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &commit_agg_pk_chunks_commitments[0],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &commit_agg_pk_chunks_commitments[1],
        // )?);
        //
        // proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
        //     &position,
        // )?);
        //
        // <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem, Fq>>::check_verify(
        //     cs.ns(|| "verify groth16 proof"),
        //     &vk_pk_tree_var,
        //     proof_inputs.iter(),
        //     &proof_var,
        // )?;
        //
        // // Verifying that the block is valid. Note that we give it the initial block number and the
        // // final public keys commitment. That's because the "state" is composed of the public keys of
        // // the current validator list and the block number of the next macro block. So, a macro block
        // // actually has the number from the previous state and contains the public keys of the next
        // // state.
        // next_cost_analysis!(cs, cost, || "Verify block");
        //
        // block_var.verify(
        //     cs.ns(|| "verify block"),
        //     &final_pk_tree_root_var,
        //     &initial_block_number_var,
        //     &prepare_agg_pk,
        //     &commit_agg_pk,
        //     &min_signers_var,
        //     &sig_generator_var,
        //     &sum_generator_g1_var,
        //     &pedersen_generators_var,
        // )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
