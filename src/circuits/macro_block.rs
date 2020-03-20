use algebra::bls12_377::{Fq, G2Projective};
use algebra::sw6::Fr as SW6Fr;
use algebra_core::{AffineCurve, ProjectiveCurve};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::bls12_377::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::*;

use crate::constants::{EPOCH_LENGTH, G1_GENERATOR1, G2_GENERATOR, MAX_NON_SIGNERS};
use crate::gadgets::{AllocConstantGadget, CRHGadgetParameters, MacroBlockGadget, StateHashGadget};
use crate::primitives::{setup_crh, MacroBlock};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

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
            &Fq::from((MAX_NON_SIGNERS + 1) as u64),
        )?;

        let sig_generator_var: G2Gadget = AllocConstantGadget::alloc_const(
            cs.ns(|| "signature generator"),
            &G2Projective::prime_subgroup_generator(),
        )?;

        let sum_generator_g1_var: G1Gadget = AllocConstantGadget::alloc_const(
            cs.ns(|| "sum generator g1"),
            &G1_GENERATOR1.clone().into_projective(),
        )?;

        let sum_generator_g2_var: G2Gadget = AllocConstantGadget::alloc_const(
            cs.ns(|| "sum generator g2"),
            &G2_GENERATOR.clone().into_projective(),
        )?;

        //This already creates a constant alloc internally.
        let crh_block_parameters_var =
            CRHGadgetParameters::alloc(&mut cs.ns(|| "CRH block parameters"), || Ok(setup_crh()))?;

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

        // Verify equality initial state hash
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

        // Verify block
        next_cost_analysis!(cs, cost, || "Verify block");

        block_var.verify(
            cs.ns(|| "verify block"),
            &prev_keys_var,
            &max_non_signers_var,
            &block_number_var,
            &sig_generator_var,
            &sum_generator_g1_var,
            &sum_generator_g2_var,
            &crh_block_parameters_var,
        )?;

        // Increment block number
        let new_block_number_var = UInt32::addmany(
            cs.ns(|| format!("increment block number")),
            &[block_number_var, epoch_length_var.clone()],
        )?;

        // Verify equality final state hash
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
