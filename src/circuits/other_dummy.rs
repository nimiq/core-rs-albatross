use algebra::bls12_377::Fr;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::prelude::*;

use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is just a circuit used for testing of the wrapper and merger circuits. It simply verifies
/// that state hashes are symmetrical.
#[derive(Clone)]
pub struct OtherDummyCircuit {
    // Public inputs
    initial_state_hash: Vec<u8>,
    final_state_hash: Vec<u8>,
}

impl OtherDummyCircuit {
    pub fn new(initial_state_hash: Vec<u8>, final_state_hash: Vec<u8>) -> Self {
        Self {
            initial_state_hash,
            final_state_hash,
        }
    }
}

impl ConstraintSynthesizer<Fr> for OtherDummyCircuit {
    fn generate_constraints<CS: ConstraintSystem<Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the public inputs.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc public inputs");

        let initial_state_hash_var = UInt8::alloc_input_vec(
            cs.ns(|| "initial state hash"),
            self.initial_state_hash.as_ref(),
        )?;

        let final_state_hash_var =
            UInt8::alloc_input_vec(cs.ns(|| "final state hash"), self.final_state_hash.as_ref())?;

        // Create variable "length", just for convenience
        let length = self.initial_state_hash.len();

        // Verify that hashes are symmetrical.
        next_cost_analysis!(cs, || "Verify inequality");
        for i in 0..length {
            initial_state_hash_var[i].enforce_equal(
                cs.ns(|| format!("initial state hash == final state hash: byte {}", i)),
                &final_state_hash_var[length - i - 1],
            )?;
        }

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
