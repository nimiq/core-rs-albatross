use ark_crypto_primitives::snark::BooleanInputVar;
use ark_crypto_primitives::SNARKGadget;
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::{Proof, VerifyingKey};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, PairingVar};
use ark_mnt6_753::{Fq, MNT6_753};
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::VKCommitmentGadget;
use crate::primitives::pedersen_generators;
use crate::utils::{pack_inputs, unpack_inputs};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is the merger circuit. It takes as inputs an initial state commitment, a final state commitment
/// and a verifying key and it produces a proof that there exist two valid SNARK proofs that transform
/// the initial state into the final state passing through some intermediate state.
/// The circuit is composed of two SNARK verifiers in a row. It's used to verify a Merger Wrapper proof
/// and a Macro Block Wrapper proof, effectively merging both into a single proof. Evidently, this is
/// needed for recursive composition of SNARK proofs.
/// This circuit has the verification key for the Macro Block Wrapper hard-coded as a constant, but the
/// verification key for the Merger Wrapper is given as a private input.
/// To guarantee that the prover inputs the correct Merger Wrapper verification key, the verifier also
/// supplies a commitment to the desired verification key as a public input. This circuit then enforces
/// the equality of the commitment and of the verification key.
/// Additionally, the prover can set (as a private input) a boolean flag determining if this circuit
/// is evaluating the genesis block or not. If the flag is set to true, the circuit will enforce that
/// the initial state and the intermediate state are equal but it will not enforce the verification of
/// the Merger Wrapper proof. If the flag is set to false, the circuit will enforce the verification
/// of the Merger Wrapper proof, but it will not enforce the equality of the initial and intermediate
/// states.
/// The rationale is that, for the genesis block, the merger circuit will not have any previous Merger
/// Wrapper proof to verify since there are no previous state changes. But in that case, the initial
/// and intermediate states must be equal by definition.
#[derive(Clone)]
pub struct MergerCircuit {
    // Verifying key for the macro block wrapper circuit. Not an input to the SNARK circuit.
    vk_macro_block_wrapper: VerifyingKey<MNT6_753>,

    // Witnesses (private)
    proof_merger_wrapper: Proof<MNT6_753>,
    proof_macro_block_wrapper: Proof<MNT6_753>,
    vk_merger_wrapper: VerifyingKey<MNT6_753>,
    intermediate_state_commitment: Vec<bool>,
    genesis_flag: bool,

    // Inputs (public)
    // Our inputs are always vectors of booleans (semantically), so that they are consistent across
    // the different elliptic curves that we use. However, for compactness, we represent them as
    // field elements. Both of the curves that we use have a modulus of 753 bits and a capacity
    // of 752 bits. So, the first 752 bits (in little-endian) of each field element is data, and the
    // last bit is always set to zero.
    initial_state_commitment: Vec<Fq>,
    final_state_commitment: Vec<Fq>,
    vk_commitment: Vec<Fq>,
}

impl MergerCircuit {
    pub fn new(
        vk_macro_block_wrapper: VerifyingKey<MNT6_753>,
        proof_merger_wrapper: Proof<MNT6_753>,
        proof_macro_block_wrapper: Proof<MNT6_753>,
        vk_merger_wrapper: VerifyingKey<MNT6_753>,
        intermediate_state_commitment: Vec<bool>,
        genesis_flag: bool,
        initial_state_commitment: Vec<Fq>,
        final_state_commitment: Vec<Fq>,
        vk_commitment: Vec<Fq>,
    ) -> Self {
        Self {
            vk_macro_block_wrapper,
            proof_merger_wrapper,
            proof_macro_block_wrapper,
            vk_merger_wrapper,
            intermediate_state_commitment,
            genesis_flag,
            initial_state_commitment,
            final_state_commitment,
            vk_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for MergerCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let pedersen_generators_var =
            Vec::<G1Var>::new_constant(cs.clone(), pedersen_generators(19))?;

        let vk_macro_block_wrapper_var = VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(
            cs.clone(),
            &self.vk_macro_block_wrapper,
        )?;

        // Allocate all the witnesses.
        next_cost_analysis!(cs, cost, || { "Alloc witnesses" });

        let proof_merger_wrapper_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || {
                Ok(&self.proof_merger_wrapper)
            })?;

        let proof_macro_block_wrapper_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || {
                Ok(&self.proof_macro_block_wrapper)
            })?;

        let vk_merger_wrapper_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || {
                Ok(&self.vk_merger_wrapper)
            })?;

        let intermediate_state_commitment_bits =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || {
                Ok(&self.intermediate_state_commitment[..])
            })?;

        let genesis_flag_var = Boolean::new_witness(cs.clone(), || Ok(&self.genesis_flag))?;

        // Allocate all the inputs.
        next_cost_analysis!(cs, cost, || { "Alloc inputs" });

        let initial_state_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.initial_state_commitment[..]))?;

        let final_state_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.final_state_commitment[..]))?;

        let vk_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.vk_commitment[..]))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        next_cost_analysis!(cs, cost, || { "Unpack inputs" });

        let initial_state_commitment_bits =
            unpack_inputs(initial_state_commitment_var)?[..760].to_vec();

        let final_state_commitment_bits =
            unpack_inputs(final_state_commitment_var)?[..760].to_vec();

        let vk_commitment_bits = unpack_inputs(vk_commitment_var)?[..760].to_vec();

        // Verify equality for vk commitment. It just checks that the private input is correct by
        // committing to it and then comparing the result with the vk commitment given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify vk commitment" });

        let reference_commitment = VKCommitmentGadget::evaluate(
            cs.clone(),
            &vk_merger_wrapper_var,
            &pedersen_generators_var,
        )?;

        vk_commitment_bits.enforce_equal(&reference_commitment)?;

        // Verify equality of initial and intermediate state commitments. If the genesis flag is set to
        // true, it enforces the equality. If it is set to false, it doesn't. This is necessary for
        // the genesis block, for the first merger circuit.
        next_cost_analysis!(cs, cost, || {
            "Conditionally verify initial and intermediate state commitments"
        });

        initial_state_commitment_bits
            .conditional_enforce_equal(&intermediate_state_commitment_bits, &genesis_flag_var)?;

        // Verify the ZK proof for the Merger Wrapper circuit. If the genesis flag is set to false,
        // it enforces the verification. If it is set to true, it doesn't. This is necessary for
        // the genesis block, for the first merger circuit.
        next_cost_analysis!(cs, cost, || { "Conditionally verify proof merger wrapper" });

        let mut proof_inputs = pack_inputs(initial_state_commitment_bits);

        proof_inputs.append(&mut pack_inputs(intermediate_state_commitment_bits.clone()));

        proof_inputs.append(&mut pack_inputs(vk_commitment_bits));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_merger_wrapper_var,
            &input_var,
            &proof_merger_wrapper_var,
        )?
        .enforce_equal(&genesis_flag_var.not())?;

        // Verify the ZK proof for the Macro Block Wrapper circuit.
        next_cost_analysis!(cs, cost, || { "Verify proof macro block wrapper" });

        let mut proof_inputs = pack_inputs(intermediate_state_commitment_bits);

        proof_inputs.append(&mut pack_inputs(final_state_commitment_bits));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_macro_block_wrapper_var,
            &input_var,
            &proof_macro_block_wrapper_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
