use algebra::bls12_377::{Fq, G2Projective};
use algebra::sw6::Fr as SW6Fr;
use algebra_core::ProjectiveCurve;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::bls12_377::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::*;

use crate::constants::{sum_generator_g1, sum_generator_g2, EPOCH_LENGTH, MAX_NON_SIGNERS};
use crate::gadgets::{AllocConstantGadget, MacroBlockGadget, StateHashGadget};
use crate::primitives::{setup_pedersen, MacroBlock};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is the macro block circuit. It takes as inputs an initial state hash and final state hash
/// and it produces a proof that there exists a valid macro block that transforms the initial state
/// into the final state.
/// Since the state is composed only of the block number and the public keys of the current validator
/// list, updating the state is just incrementing the block number and substituting the previous
/// public keys with the public keys of the new validator list.
#[derive(Clone)]
pub struct MacroBlockCircuit {
    // Private inputs
    prev_keys: Vec<G2Projective>,
    block_number: u32,
    block: MacroBlock,

    // Public inputs
    initial_state_hash: Vec<u8>,
    final_state_hash: Vec<u8>,
}

impl MacroBlockCircuit {
    pub fn new(
        prev_keys: Vec<G2Projective>,
        block_number: u32,
        block: MacroBlock,
        initial_state_hash: Vec<u8>,
        final_state_hash: Vec<u8>,
    ) -> Self {
        Self {
            prev_keys,
            block_number,
            block,
            initial_state_hash,
            final_state_hash,
        }
    }
}

impl ConstraintSynthesizer<SW6Fr> for MacroBlockCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints<CS: ConstraintSystem<SW6Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let epoch_length_var = UInt32::constant(EPOCH_LENGTH);

        let max_non_signers_var: FqGadget = AllocConstantGadget::alloc_const(
            cs.ns(|| "max non signers"),
            &Fq::from(MAX_NON_SIGNERS as u64),
        )?;

        let sig_generator_var: G2Gadget = AllocConstantGadget::alloc_const(
            cs.ns(|| "signature generator"),
            &G2Projective::prime_subgroup_generator(),
        )?;

        let sum_generator_g1_var: G1Gadget =
            AllocConstantGadget::alloc_const(cs.ns(|| "sum generator g1"), &sum_generator_g1())?;

        let sum_generator_g2_var: G2Gadget =
            AllocConstantGadget::alloc_const(cs.ns(|| "sum generator g2"), &sum_generator_g2())?;

        let pedersen_generators = setup_pedersen();
        let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
        for i in 0..pedersen_generators.len() {
            pedersen_generators_var.push(AllocConstantGadget::alloc_const(
                cs.ns(|| format!("pedersen_generators: generator {}", i)),
                &pedersen_generators[i],
            )?);
        }

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || "Alloc private inputs");

        let mut prev_keys_var = Vec::new();
        for i in 0..self.prev_keys.len() {
            prev_keys_var.push(G2Gadget::alloc(
                cs.ns(|| format!("previous keys: key {}", i)),
                || Ok(&self.prev_keys[i]),
            )?);
        }

        let block_number_var = UInt32::alloc(cs.ns(|| "block number"), Some(self.block_number))?;

        let block_var = MacroBlockGadget::alloc(cs.ns(|| "macro block"), || Ok(&self.block))?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });

        let initial_state_hash_var = UInt8::alloc_input_vec(
            cs.ns(|| "initial state hash"),
            self.initial_state_hash.as_ref(),
        )?;

        let final_state_hash_var =
            UInt8::alloc_input_vec(cs.ns(|| "final state hash"), self.final_state_hash.as_ref())?;

        // Verifying equality for initial state hash. It just checks that the private inputs are correct
        // by hashing them and comparing the result with the initial state hash given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify initial state hash" });

        let reference_hash = StateHashGadget::evaluate(
            cs.ns(|| "reference initial state hash"),
            &block_number_var,
            &prev_keys_var,
        )?;

        for i in 0..32 {
            initial_state_hash_var[i].enforce_equal(
                cs.ns(|| format!("initial state hash == reference hash: byte {}", i)),
                &reference_hash[i],
            )?;
        }

        // Verifying block that the block is valid. Indirectly, this also allows us to know the
        // next validator list public keys.
        next_cost_analysis!(cs, cost, || "Verify block");

        block_var.verify(
            cs.ns(|| "verify block"),
            &prev_keys_var,
            &max_non_signers_var,
            &block_number_var,
            &sig_generator_var,
            &sum_generator_g1_var,
            &sum_generator_g2_var,
            &pedersen_generators_var,
        )?;

        // Incrementing the block number.
        let new_block_number_var = UInt32::addmany(
            cs.ns(|| format!("increment block number")),
            &[block_number_var, epoch_length_var.clone()],
        )?;

        // Verifying equality for final state hash. It just checks that the internal results are
        // indeed equal to the final state hash given as a public input.
        next_cost_analysis!(cs, cost, || { "Verify final state hash" });

        let reference_hash = StateHashGadget::evaluate(
            cs.ns(|| "reference final state hash"),
            &new_block_number_var,
            &block_var.public_keys,
        )?;

        for i in 0..32 {
            final_state_hash_var[i].enforce_equal(
                cs.ns(|| format!("final state hash == reference hash: byte {}", i)),
                &reference_hash[i],
            )?;
        }

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
