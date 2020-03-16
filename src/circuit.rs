use algebra::bls12_377::{Fq, G2Projective, Parameters as Bls12_377Parameters};
use algebra::sw6::Fr as SW6Fr;
use algebra_core::{AffineCurve, ProjectiveCurve};
use crypto_primitives::FixedLengthCRHGadget;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::{G1Gadget, G2Gadget};
use r1cs_std::prelude::*;

use crate::constants::{EPOCH_LENGTH, G1_GENERATOR1, G2_GENERATOR, MAX_NON_SIGNERS};
use crate::gadgets::crh::{setup_crh, CRHGadget, CRHWindow, CRH};
use crate::gadgets::{
    alloc_constant::AllocConstantGadget, macro_block::MacroBlockGadget,
    state_hash::calculate_state_hash,
};
use crate::macro_block::MacroBlock;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

pub struct Circuit {
    // Private inputs
    prev_keys: Vec<G2Projective>,
    block_number: u32,
    block: MacroBlock,

    // Public inputs
    initial_state_hash: Vec<u32>,
    final_state_hash: Vec<u32>,
}

impl Circuit {
    pub fn new(
        prev_keys: Vec<G2Projective>,
        block_number: u32,
        block: MacroBlock,
        initial_state_hash: Vec<u32>,
        final_state_hash: Vec<u32>,
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

impl ConstraintSynthesizer<Fq> for Circuit {
    fn generate_constraints<CS: ConstraintSystem<Fq>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");
        let epoch_length_var = UInt32::constant(EPOCH_LENGTH);
        let max_non_signers_var: FpGadget<SW6Fr> = AllocConstantGadget::alloc_const(
            cs.ns(|| "max non signers"),
            &SW6Fr::from(MAX_NON_SIGNERS),
        )?;
        let sig_generator_var: G2Gadget<Bls12_377Parameters> = AllocConstantGadget::alloc_const(
            cs.ns(|| "signature generator"),
            &G2Projective::prime_subgroup_generator(),
        )?;
        let sum_generator_g1_var: G1Gadget<Bls12_377Parameters> = AllocConstantGadget::alloc_const(
            cs.ns(|| "sum generator g1"),
            &G1_GENERATOR1.clone().into_projective(),
        )?;
        let sum_generator_g2_var: G2Gadget<Bls12_377Parameters> = AllocConstantGadget::alloc_const(
            cs.ns(|| "sum generator g2"),
            &G2_GENERATOR.clone().into_projective(),
        )?;
        let crh_block_parameters_var =
            <CRHGadget as FixedLengthCRHGadget<CRH<CRHWindow>, SW6Fr>>::ParametersGadget::alloc(
                &mut cs.ns(|| "CRH block parameters"),
                || Ok(setup_crh::<CRHWindow>()),
            )?;

        // Allocate all the private inputs.
        next_cost_analysis!(cs, cost, || "Alloc private inputs");
        let mut prev_keys_var = Vec::new();
        for i in 0..self.prev_keys.len() {
            prev_keys_var.push(G2Gadget::<Bls12_377Parameters>::alloc(
                cs.ns(|| format!("previous keys: key {}", i)),
                || Ok(&self.prev_keys[i]),
            )?);
        }

        let block_number_var = UInt32::alloc(cs.ns(|| "block number"), Some(self.block_number))?;

        let block_var = MacroBlockGadget::alloc(cs.ns(|| "macro blocks"), || Ok(&self.block))?;

        // Allocate all the public inputs.
        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });
        let mut initial_state_hash_var = Vec::new();
        for i in 0..8 {
            initial_state_hash_var.push(UInt32::alloc_input(
                cs.ns(|| format!("state hash byte {}", i)),
                Some(self.initial_state_hash[i]),
            )?);
        }

        let mut final_state_hash_var = Vec::new();
        for i in 0..8 {
            final_state_hash_var.push(UInt32::alloc_input(
                cs.ns(|| format!("state hash byte {}", i)),
                Some(self.final_state_hash[i]),
            )?);
        }

        // Verify equality initial state hash
        next_cost_analysis!(cs, cost, || { "Verify initial state hash" });
        let reference_hash = calculate_state_hash(
            cs.ns(|| "calculate initial state hash"),
            &block_number_var,
            &prev_keys_var,
        )?;
        for i in 0..8 {
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
        let reference_hash = calculate_state_hash(
            cs.ns(|| "calculate final state hash"),
            &new_block_number_var,
            &block_var.public_keys,
        )?;
        for i in 0..8 {
            final_state_hash_var[i].enforce_equal(
                cs.ns(|| format!("final state hash == reference hash: byte {}", i)),
                &reference_hash[i],
            )?;
        }

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
