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

    // Public input
    last_public_keys: Vec<G2Projective>,
}

impl Benchmark {
    pub fn new(
        genesis_keys: Vec<G2Projective>,
        test_block: MacroBlock,
        generator: G2Projective,
        max_non_signers: u64,
        last_public_keys: Vec<G2Projective>,
    ) -> Self {
        Self {
            genesis_keys,
            test_block,
            generator,
            max_non_signers,
            last_public_keys,
        }
    }
}

impl ConstraintSynthesizer<Fq> for Benchmark {
    fn generate_constraints<CS: ConstraintSystem<Fq>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        assert_eq!(self.last_public_keys.len(), MacroBlock::SLOTS);

        let last_public_keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc_input(
            cs.ns(|| "last public keys"),
            || Ok(&self.last_public_keys[..]),
        )?;

        let genesis_keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc_const(
            cs.ns(|| "genesis keys"),
            &self.genesis_keys[..],
        )?;

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

        let last_block_var = block_var;

        // Finally verify that the last block's public keys correspond to the public input.
        // This assumes giving them in exactly the same order.
        for (i, (key1, key2)) in last_public_keys_var
            .iter()
            .zip(last_block_var.public_keys.iter())
            .enumerate()
        {
            key1.enforce_equal(cs.ns(|| format!("public key equality {}", i)), key2)?;
        }

        Ok(())
    }
}
