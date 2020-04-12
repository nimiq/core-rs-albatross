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

use crate::circuits::mnt4::DummyCircuit;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the wrapper circuit just by editing these types.
type TheProofSystem = Groth16<MNT4_753, DummyCircuit, Fr>;
type TheProofGadget = ProofGadget<MNT4_753, Fq, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT4_753, Fq, PairingGadget>;
type TheVerifierGadget = Groth16VerifierGadget<MNT4_753, Fq, PairingGadget>;

/// This is the wrapper circuit. It takes as inputs an initial state hash, a final state hash and a
/// verifying key and it produces a proof that there exists a valid SNARK proof that transforms the
/// initial state into the final state.
/// The circuit is basically only a SNARK verifier. Its use is just to change the elliptic curve
/// that the proof exists in, which is sometimes needed for recursive composition of SNARK proofs.
/// This circuit is not safe to use because it has the verifying key as a private input. In production
/// this needs to be a constant, so we'll have different wrapper circuits (one for each inner circuit
/// that is being verified).
#[derive(Clone)]
pub struct WrapperCircuit {
    // Private inputs
    proof: Proof<MNT4_753>,
    verifying_key: VerifyingKey<MNT4_753>,

    // Public inputs
    initial_state_hash: Vec<u8>,
    final_state_hash: Vec<u8>,
}

impl WrapperCircuit {
    pub fn new(
        proof: Proof<MNT4_753>,
        verifying_key: VerifyingKey<MNT4_753>,
        initial_state_hash: Vec<u8>,
        final_state_hash: Vec<u8>,
    ) -> Self {
        Self {
            proof,
            verifying_key,
            initial_state_hash,
            final_state_hash,
        }
    }
}

impl ConstraintSynthesizer<MNT6Fr> for WrapperCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<MNT6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the private inputs.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc private inputs");

        let proof_var = TheProofGadget::alloc(cs.ns(|| "alloc proof"), || Ok(&self.proof))?;

        let verifying_key_var =
            TheVkGadget::alloc(cs.ns(|| "alloc verifying key"), || Ok(&self.verifying_key))?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let initial_state_hash_var = UInt8::alloc_input_vec(
            cs.ns(|| "initial state hash"),
            self.initial_state_hash.as_ref(),
        )?;

        let final_state_hash_var =
            UInt8::alloc_input_vec(cs.ns(|| "final state hash"), self.final_state_hash.as_ref())?;

        // Verify the ZK proof.
        next_cost_analysis!(cs, cost, || { "Verify ZK proof" });
        let mut proof_inputs = vec![];
        proof_inputs.push(initial_state_hash_var);
        proof_inputs.push(final_state_hash_var);

        <TheVerifierGadget as NIZKVerifierGadget<TheProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify groth16 proof"),
            &verifying_key_var,
            proof_inputs.iter(),
            &proof_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
