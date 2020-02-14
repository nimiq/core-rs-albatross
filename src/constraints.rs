use algebra::curves::bls12_377::{Bls12_377Parameters, G2Projective};
use algebra::fields::bls12_377::fq::Fq;
use algebra::fields::sw6::Fr as SW6Fr;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::G2Gadget;
use r1cs_std::prelude::*;

use crate::gadgets::constant::AllocConstantGadget;
use crate::gadgets::macro_block::{MacroBlock, MacroBlockGadget};

pub struct Circuit {
    genesis_keys: Vec<G2Projective>,
    blocks: Vec<MacroBlock>,
    generator: G2Projective,
    max_non_signers: u64,
    block_flags: Vec<bool>,

    // Public input
    last_public_keys: Vec<G2Projective>,
}

impl Circuit {
    pub const MAX_BLOCKS: usize = 3;

    pub fn new(
        genesis_keys: Vec<G2Projective>,
        mut blocks: Vec<MacroBlock>,
        generator: G2Projective,
        max_non_signers: u64,
        last_public_keys: Vec<G2Projective>,
    ) -> Self {
        let num_blocks = blocks.len();
        assert!(num_blocks > 0, "Must verify at least one block");
        let mut block_flags = vec![true; num_blocks - 1];

        // Fill up dummy data for rest of the blocks.
        for _ in num_blocks..Self::MAX_BLOCKS {
            blocks.push(Default::default());
            block_flags.push(false);
        }

        Self {
            genesis_keys,
            blocks,
            generator,
            max_non_signers,
            last_public_keys,
            block_flags,
        }
    }
}

impl ConstraintSynthesizer<Fq> for Circuit {
    fn generate_constraints<CS: ConstraintSystem<Fq>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        assert_eq!(self.last_public_keys.len(), MacroBlock::SLOTS);
        assert_eq!(self.block_flags.len(), Self::MAX_BLOCKS - 1);
        assert_eq!(self.blocks.len(), Self::MAX_BLOCKS);

        let last_public_keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc_input(
            cs.ns(|| "last public keys"),
            || Ok(&self.last_public_keys[..]),
        )?;

        let block_flags_var =
            Vec::<Boolean>::alloc(cs.ns(|| "block flags"), || Ok(&self.block_flags[..]))?;

        let genesis_keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc_const(
            cs.ns(|| "genesis keys"),
            &self.genesis_keys[..],
        )?;

        let mut blocks_var =
            Vec::<MacroBlockGadget>::alloc(cs.ns(|| "macro blocks"), || Ok(&self.blocks[..]))?;

        let generator_var: G2Gadget<Bls12_377Parameters> =
            AllocConstantGadget::alloc_const(cs.ns(|| "generator"), &self.generator)?;

        let max_non_signers_var: FpGadget<SW6Fr> = AllocConstantGadget::alloc_const(
            cs.ns(|| "max non signers"),
            &SW6Fr::from(self.max_non_signers),
        )?;

        // We always require at least one block to be verified.
        // This block then also provides the initial values for conditional selects during
        // the rest of the circuit. One example is the last block's list of public keys.
        let first_block_var = blocks_var.remove(0);
        let mut prev_public_keys = first_block_var.conditional_verify(
            cs.ns(|| "verify block 0"),
            &genesis_keys_var,
            &max_non_signers_var,
            &generator_var,
            &Boolean::constant(true),
        )?;

        // The verification starts with true and is recalculated for each block as
        // `verification_flag & block_flag`, where `block_flag` denotes the bit indicating
        // whether the current block needs to be checked.
        // After this calculation, we enforce `verification_flag == block_flag`.
        // This enforces that once a block flag was set to false, no following block flag might
        // ever be set to true again.
        let mut verification_flag = Boolean::constant(true);
        for (i, (block, block_flag)) in blocks_var.iter().zip(block_flags_var.iter()).enumerate() {
            // Enforce verification.
            verification_flag = Boolean::and(
                cs.ns(|| format!("verification flag {}", i + 1)),
                &verification_flag,
                block_flag,
            )?;
            verification_flag.enforce_equal(
                cs.ns(|| format!("verification flag == block flag {}", i + 1)),
                block_flag,
            )?;

            prev_public_keys = block.conditional_verify(
                cs.ns(|| format!("verify block {}", i + 1)),
                &prev_public_keys,
                &max_non_signers_var,
                &generator_var,
                &block_flag,
            )?;
        }

        // Finally verify that the last block's public keys correspond to the public input.
        // This assumes giving them in exactly the same order.
        for (i, (key1, key2)) in last_public_keys_var
            .iter()
            .zip(prev_public_keys.iter())
            .enumerate()
        {
            key1.enforce_equal(cs.ns(|| format!("public key equality {}", i)), key2)?;
        }

        Ok(())
    }
}
