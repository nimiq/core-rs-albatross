use algebra::curves::bls12_377::{Bls12_377Parameters, G2Projective};
use algebra::fields::bls12_377::fq::Fq;
use algebra::fields::sw6::Fr as SW6Fr;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::G2Gadget;
use r1cs_std::prelude::*;

use crate::gadgets::constant::AllocConstantGadget;
use crate::gadgets::macro_block::{MacroBlock, MacroBlockGadget};

pub struct Benchmark {
    genesis_keys: Vec<G2Projective>,
    test_block: MacroBlock,
    generator: G2Projective,
    max_non_signers: u64,
}

impl Benchmark {
    pub fn new(
        genesis_keys: Vec<G2Projective>,
        test_block: MacroBlock,
        generator: G2Projective,
        max_non_signers: u64,
    ) -> Self {
        Self {
            genesis_keys,
            test_block,
            generator,
            max_non_signers,
        }
    }
}

impl ConstraintSynthesizer<Fq> for Benchmark {
    fn generate_constraints<CS: ConstraintSystem<Fq>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        let genesis_keys_var =
            Vec::<G2Gadget<Bls12_377Parameters>>::alloc(cs.ns(|| "genesis keys"), || {
                Ok(&self.genesis_keys[..])
            })?;

        let block_var =
            MacroBlockGadget::alloc(cs.ns(|| "first macro block"), || Ok(&self.test_block))?;

        let generator_var: G2Gadget<Bls12_377Parameters> =
            AllocConstantGadget::alloc_const(cs.ns(|| "generator"), &self.generator)?;

        let max_non_signers_var: FpGadget<SW6Fr> = AllocConstantGadget::alloc_const(
            cs.ns(|| "max non signers"),
            &SW6Fr::from(self.max_non_signers),
        )?;

        block_var.verify(
            cs.ns(|| "verify block"),
            &genesis_keys_var,
            &max_non_signers_var,
            &generator_var,
        )?;

        Ok(())
    }
}
