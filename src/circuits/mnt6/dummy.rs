use algebra::mnt6_753::Fr as MNT6Fr;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::prelude::*;

use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is just a circuit used for testing of the wrapper and merger circuits. It simply verifies
/// that the inputs are equal.
#[derive(Clone)]
pub struct DummyCircuit {
    // Public inputs
    first_input: Vec<u8>,
    second_input: Vec<u8>,
}

impl DummyCircuit {
    pub fn new(first_input: Vec<u8>, second_input: Vec<u8>) -> Self {
        Self {
            first_input,
            second_input,
        }
    }
}

impl ConstraintSynthesizer<MNT6Fr> for DummyCircuit {
    fn generate_constraints<CS: ConstraintSystem<MNT6Fr>>(
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

        // Verify equality.
        next_cost_analysis!(cs, || "Verify equality");
        for i in 0..self.first_input.len() {
            first_input_var[i].enforce_equal(
                cs.ns(|| format!("first input == second input: byte {}", i)),
                &second_input_var[i],
            )?;
        }

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
