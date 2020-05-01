use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fq, Fr, MNT6_753};
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::groth16::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::{G1Gadget, PairingGadget};
use r1cs_std::prelude::*;

use crate::circuits::mnt6::{MacroBlockWrapperCircuit, MergerWrapperCircuit};
use crate::constants::sum_generator_g1_mnt6;
use crate::gadgets::input::RecursiveInputGadget;
use crate::gadgets::mnt4::VKCommitmentGadget;
use crate::primitives::pedersen_generators;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the merger circuit just by editing these types.
type FirstProofSystem = Groth16<MNT6_753, MergerWrapperCircuit, Fr>;
type SecondProofSystem = Groth16<MNT6_753, MacroBlockWrapperCircuit, Fr>;
type TheProofGadget = ProofGadget<MNT6_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT6_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT6_753, Fq, PairingGadget>;

/// This is the merger circuit. It takes as inputs an initial state commitment, a final state commitment
/// and a verifying key and it produces a proof that there exists two valid SNARK proofs that transforms
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
    // Private inputs
    proof_merger_wrapper: Proof<MNT6_753>,
    proof_macro_block_wrapper: Proof<MNT6_753>,
    vk_merger_wrapper: VerifyingKey<MNT6_753>,
    vk_macro_block_wrapper: VerifyingKey<MNT6_753>,
    intermediate_state_commitment: Vec<u8>,
    genesis_flag: bool,

    // Public inputs
    initial_state_commitment: Vec<u8>,
    final_state_commitment: Vec<u8>,
    vk_commitment: Vec<u8>,
}

impl MergerCircuit {
    pub fn new(
        proof_merger_wrapper: Proof<MNT6_753>,
        proof_macro_block_wrapper: Proof<MNT6_753>,
        vk_merger_wrapper: VerifyingKey<MNT6_753>,
        vk_macro_block_wrapper: VerifyingKey<MNT6_753>,
        intermediate_state_commitment: Vec<u8>,
        genesis_flag: bool,
        initial_state_commitment: Vec<u8>,
        final_state_commitment: Vec<u8>,
        vk_commitment: Vec<u8>,
    ) -> Self {
        Self {
            proof_merger_wrapper,
            proof_macro_block_wrapper,
            vk_merger_wrapper,
            vk_macro_block_wrapper,
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
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let sum_generator_g1_var =
            G1Gadget::alloc_constant(cs.ns(|| "alloc sum generator g1"), &sum_generator_g1_mnt6())?;

        let pedersen_generators = pedersen_generators(17);

        let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();

        for i in 0..pedersen_generators.len() {
            pedersen_generators_var.push(G1Gadget::alloc_constant(
                cs.ns(|| format!("alloc pedersen_generators: generator {}", i)),
                &pedersen_generators[i],
            )?);
        }

        // TODO: This needs to be changed to a constant!
        let vk_macro_block_wrapper_var =
            TheVkGadget::alloc(cs.ns(|| "alloc vk macro block wrapper"), || {
                Ok(&self.vk_macro_block_wrapper)
            })?;

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || { "Alloc private inputs" });

        let proof_merger_wrapper_var =
            TheProofGadget::alloc(cs.ns(|| "alloc proof merger wrapper"), || {
                Ok(&self.proof_merger_wrapper)
            })?;

        let proof_macro_block_wrapper_var =
            TheProofGadget::alloc(cs.ns(|| "alloc proof macro block wrapper"), || {
                Ok(&self.proof_macro_block_wrapper)
            })?;

        let vk_merger_wrapper_var =
            TheVkGadget::alloc(cs.ns(|| "alloc vk merger wrapper"), || {
                Ok(&self.vk_merger_wrapper)
            })?;

        let intermediate_state_commitment_var = UInt8::alloc_vec(
            cs.ns(|| "alloc intermediate state commitment"),
            self.intermediate_state_commitment.as_ref(),
        )?;

        let genesis_flag_var =
            Boolean::alloc(cs.ns(|| "alloc genesis flag"), || Ok(self.genesis_flag))?;

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

        let vk_commitment_var =
            UInt8::alloc_input_vec(cs.ns(|| "alloc vk commitment"), self.vk_commitment.as_ref())?;

        // Verify equality for vk commitment. It just checks that the private input is correct by
        // committing to it and then comparing the result with the vk commitment given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify vk commitment" });

        let reference_commitment = VKCommitmentGadget::evaluate(
            cs.ns(|| "reference vk commitment"),
            &vk_merger_wrapper_var,
            &pedersen_generators_var,
            &sum_generator_g1_var,
        )?;

        vk_commitment_var.enforce_equal(
            cs.ns(|| "vk commitment == reference commitment"),
            &reference_commitment,
        )?;

        // Verify equality of initial and intermediate state commitments. If the genesis flag is set to
        // true, it enforces the equality. If it is set to false, it doesn't. This is necessary for
        // the genesis block, for the first merger circuit.
        next_cost_analysis!(cs, cost, || {
            "Conditionally verify initial and intermediate state commitments"
        });

        initial_state_commitment_var.conditional_enforce_equal(
            cs.ns(|| "initial state commitment == intermediate state commitment"),
            &intermediate_state_commitment_var,
            &genesis_flag_var,
        )?;

        // Verify the ZK proof for the Merger Wrapper circuit. If the genesis flag is set to false,
        // it enforces the verification. If it is set to true, it doesn't. This is necessary for
        // the genesis block, for the first merger circuit.
        next_cost_analysis!(cs, cost, || { "Conditionally verify proof merger wrapper" });

        let mut proof_inputs =
            RecursiveInputGadget::to_field_elements::<Fr>(&initial_state_commitment_var)?;

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &intermediate_state_commitment_var,
        )?);

        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &vk_commitment_var,
        )?);

        let neg_genesis_flag_var = genesis_flag_var.not();

        <TheVerifierGadget as NIZKVerifierGadget<FirstProofSystem, Fq>>::conditional_check_verify(
            cs.ns(|| "verify merger wrapper groth16 proof"),
            &vk_merger_wrapper_var,
            proof_inputs.iter(),
            &proof_merger_wrapper_var,
            &neg_genesis_flag_var,
        )?;

        // Verify the ZK proof for the Macro Block Wrapper circuit.
        next_cost_analysis!(cs, cost, || { "Verify proof macro block wrapper" });

        let mut proof_inputs =
            RecursiveInputGadget::to_field_elements::<Fr>(&intermediate_state_commitment_var)?;
        proof_inputs.append(&mut RecursiveInputGadget::to_field_elements::<Fr>(
            &final_state_commitment_var,
        )?);

        <TheVerifierGadget as NIZKVerifierGadget<SecondProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify macro block wrapper groth16 proof"),
            &vk_macro_block_wrapper_var,
            proof_inputs.iter(),
            &proof_macro_block_wrapper_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
