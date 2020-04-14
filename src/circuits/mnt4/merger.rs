use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fq, Fr, MNT6_753};
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::groth16::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::PairingGadget;
use r1cs_std::prelude::*;

use crate::circuits::mnt6::{DummyCircuit, OtherDummyCircuit};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the merger circuit just by editing these types.
type FirstProofSystem = Groth16<MNT6_753, DummyCircuit, Fr>;
type SecondProofSystem = Groth16<MNT6_753, OtherDummyCircuit, Fr>;
type TheProofGadget = ProofGadget<MNT6_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT6_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT6_753, Fq, PairingGadget>;

/// This is the merger circuit. It takes as inputs an initial state commitment, a final state commitment and a
/// verifying key and it produces a proof that there exists two valid SNARK proofs that transforms the
/// initial state into the final state passing through some intermediate state.
/// The circuit is composed of two SNARK verifiers in a row. It's used to merge two SNARK proofs
/// (normally two wrapper proofs) into a single proof. Evidently, this is needed for recursive
/// composition of SNARK proofs.
/// This circuit is not safe to use because it has the verifying key as a private input. In production
/// this needs to be a constant, so we'll have different wrapper circuits (one for each inner circuit
/// that is being verified).
#[derive(Clone)]
pub struct MergerCircuit {
    // Private inputs
    proof_1: Proof<MNT6_753>,
    proof_2: Proof<MNT6_753>,
    verifying_key_1: VerifyingKey<MNT6_753>,
    verifying_key_2: VerifyingKey<MNT6_753>,
    intermediate_state_commitment: Vec<u8>,

    // Public inputs
    initial_state_commitment: Vec<u8>,
    final_state_commitment: Vec<u8>,
}

impl MergerCircuit {
    pub fn new(
        proof_1: Proof<MNT6_753>,
        proof_2: Proof<MNT6_753>,
        verifying_key_1: VerifyingKey<MNT6_753>,
        verifying_key_2: VerifyingKey<MNT6_753>,
        intermediate_state_commitment: Vec<u8>,
        initial_state_commitment: Vec<u8>,
        final_state_commitment: Vec<u8>,
    ) -> Self {
        Self {
            proof_1,
            proof_2,
            verifying_key_1,
            verifying_key_2,
            intermediate_state_commitment,
            initial_state_commitment,
            final_state_commitment,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for MergerCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the private inputs.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc private inputs");

        let proof_1_var =
            TheProofGadget::alloc(cs.ns(|| "alloc first proof"), || Ok(&self.proof_1))?;

        let proof_2_var =
            TheProofGadget::alloc(cs.ns(|| "alloc second proof"), || Ok(&self.proof_2))?;

        let intermediate_state_commitment_var = UInt8::alloc_vec(
            cs.ns(|| "intermediate state commitment"),
            self.intermediate_state_commitment.as_ref(),
        )?;

        let verifying_key_var_1 =
            TheVkGadget::alloc(cs.ns(|| "alloc first verifying key"), || {
                Ok(&self.verifying_key_1)
            })?;

        let verifying_key_var_2 =
            TheVkGadget::alloc(cs.ns(|| "alloc second verifying key"), || {
                Ok(&self.verifying_key_2)
            })?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let initial_state_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "initial state commitment"),
            self.initial_state_commitment.as_ref(),
        )?;

        let final_state_commitment_var = UInt8::alloc_input_vec(
            cs.ns(|| "final state commitment"),
            self.final_state_commitment.as_ref(),
        )?;

        // Verify the first ZK proof.
        next_cost_analysis!(cs, cost, || { "Verify first ZK proof" });
        let mut proof_inputs = vec![];
        proof_inputs.push(initial_state_commitment_var);
        proof_inputs.push(intermediate_state_commitment_var.clone());

        <TheVerifierGadget as NIZKVerifierGadget<FirstProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify first groth16 proof"),
            &verifying_key_var_1,
            proof_inputs.iter(),
            &proof_1_var,
        )?;

        // Verify the second ZK proof.
        next_cost_analysis!(cs, cost, || { "Verify second ZK proof" });
        let mut proof_inputs = vec![];
        proof_inputs.push(intermediate_state_commitment_var);
        proof_inputs.push(final_state_commitment_var);

        <TheVerifierGadget as NIZKVerifierGadget<SecondProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify second groth16 proof"),
            &verifying_key_var_2,
            proof_inputs.iter(),
            &proof_2_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
