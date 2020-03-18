use algebra::bls12_377::{Fq, Fr};
use algebra::sw6::Fr as SW6Fr;
use algebra::Bls12_377;
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::groth16::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{Proof, VerifyingKey};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::bls12_377::PairingGadget;
use r1cs_std::prelude::*;

use crate::circuits::DummyCircuit;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

type MyProofSystem = Groth16<Bls12_377, DummyCircuit, Fr>;
type MyProofGadget = ProofGadget<Bls12_377, Fq, PairingGadget>;
type MyVkGadget = VerifyingKeyGadget<Bls12_377, Fq, PairingGadget>;
type MyVerifierGadget = Groth16VerifierGadget<Bls12_377, Fq, PairingGadget>;

pub struct WrapperCircuit {
    // Private inputs
    proof: Proof<Bls12_377>,

    // Public inputs
    verifying_key: VerifyingKey<Bls12_377>,
    initial_state_hash: Vec<u8>,
    final_state_hash: Vec<u8>,
}

impl WrapperCircuit {
    pub fn new(
        proof: Proof<Bls12_377>,
        verifying_key: VerifyingKey<Bls12_377>,
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

impl ConstraintSynthesizer<SW6Fr> for WrapperCircuit {
    fn generate_constraints<CS: ConstraintSystem<SW6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the private inputs.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc private inputs");

        let proof_var = MyProofGadget::alloc(cs.ns(|| "alloc proof"), || Ok(&self.proof))?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let verifying_key_var =
            MyVkGadget::alloc_input(cs.ns(|| "alloc verifying key"), || Ok(&self.verifying_key))?;

        let initial_state_hash_var = UInt8::alloc_input_vec(
            cs.ns(|| "initial state hash"),
            self.initial_state_hash.as_ref(),
        )?;

        let final_state_hash_var =
            UInt8::alloc_input_vec(cs.ns(|| "final state hash"), self.final_state_hash.as_ref())?;

        // Verify the ZK proof.
        next_cost_analysis!(cs, cost, || { "Verify ZK proof" });
        let mut proof_inputs = vec![];
        proof_inputs.extend(initial_state_hash_var);
        proof_inputs.extend(final_state_hash_var);

        <MyVerifierGadget as NIZKVerifierGadget<MyProofSystem, Fq>>::check_verify(
            cs.ns(|| "verify groth16 proof"),
            &verifying_key_var,
            proof_inputs.iter(),
            &proof_var,
        )?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
