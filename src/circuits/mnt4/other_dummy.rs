use algebra::mnt4_753::Fr as MNT4Fr;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::prelude::*;

use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is just a circuit used for testing of the wrapper and merger circuits. It simply verifies
/// that the inputs are symmetrical.
#[derive(Clone)]
pub struct OtherDummyCircuit {
    // Public inputs
    first_input: Vec<u8>,
    second_input: Vec<u8>,
}

impl OtherDummyCircuit {
    pub fn new(first_input: Vec<u8>, second_input: Vec<u8>) -> Self {
        Self {
            first_input: first_input,
            second_input: second_input,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for OtherDummyCircuit {
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the public inputs.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc public inputs");

        let first_input_var =
            UInt8::alloc_input_vec(cs.ns(|| "alloc first input"), self.first_input.as_ref())?;

        let second_input_var =
            UInt8::alloc_input_vec(cs.ns(|| "alloc second input"), self.second_input.as_ref())?;

        // Verify that hashes are symmetrical.
        next_cost_analysis!(cs, || "Verify symmetry");
        for i in 0..self.first_input.len() {
            first_input_var[i].enforce_equal(
                cs.ns(|| format!("first input == second input: byte {}", i)),
                &second_input_var[self.first_input.len() - i - 1],
            )?;
        }

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
