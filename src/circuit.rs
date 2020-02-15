use algebra::curves::bls12_377::{Bls12_377Parameters, G2Projective};
use algebra::fields::bls12_377::fq::Fq;
use algebra::fields::sw6::Fr as SW6Fr;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::G2Gadget;
use r1cs_std::prelude::*;
use std::borrow::Cow;

use crate::gadgets::constant::AllocConstantGadget;
use crate::gadgets::macro_block::MacroBlockGadget;
use crate::macro_block::MacroBlock;

pub struct Circuit {
    max_blocks: usize,
    genesis_keys: Vec<G2Projective>,
    blocks: Vec<MacroBlock>,
    generator: G2Projective,
    max_non_signers: u64,
    block_flags: Vec<bool>,

    // Public input
    last_public_key_sum: G2Projective,
}

impl Circuit {
    pub const EPOCH_LENGTH: u32 = 10;

    /// `min_signers` enforces the number of signers to be >= `min_signers`.
    pub fn new(
        max_blocks: usize,
        genesis_keys: Vec<G2Projective>,
        mut blocks: Vec<MacroBlock>,
        generator: G2Projective,
        min_signers: usize,
        last_public_key_sum: G2Projective,
    ) -> Self {
        let num_blocks = blocks.len();
        assert!(num_blocks > 0, "Must verify at least one block");
        let mut block_flags = vec![true; num_blocks - 1];

        // Fill up dummy data for rest of the blocks.
        for _ in num_blocks..max_blocks {
            blocks.push(Default::default());
            block_flags.push(false);
        }

        Self {
            max_blocks,
            genesis_keys,
            blocks,
            generator,
            // non < max_excl <=> signers >= min_incl => max_excl = (SLOTS - min_incl + 1)
            max_non_signers: (MacroBlock::SLOTS - min_signers + 1) as u64,
            last_public_key_sum,
            block_flags,
        }
    }
}

impl ConstraintSynthesizer<Fq> for Circuit {
    fn generate_constraints<CS: ConstraintSystem<Fq>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        assert_eq!(self.block_flags.len(), self.max_blocks - 1);
        assert_eq!(self.blocks.len(), self.max_blocks);

        let last_public_key_sum_var =
            G2Gadget::<Bls12_377Parameters>::alloc_input(cs.ns(|| "last public key sum"), || {
                Ok(&self.last_public_key_sum)
            })?;

        let block_flags_var =
            Vec::<Boolean>::alloc(cs.ns(|| "block flags"), || Ok(&self.block_flags[..]))?;

        let genesis_keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc_const(
            cs.ns(|| "genesis keys"),
            &self.genesis_keys[..],
        )?;
        // The block number is part of the hash.
        let epoch_length = UInt32::constant(Self::EPOCH_LENGTH);

        let mut blocks_var =
            Vec::<MacroBlockGadget>::alloc(cs.ns(|| "macro blocks"), || Ok(&self.blocks[..]))?;

        let generator_var: G2Gadget<Bls12_377Parameters> =
            AllocConstantGadget::alloc_const(cs.ns(|| "generator"), &self.generator)?;

        let max_non_signers_var: FpGadget<SW6Fr> = AllocConstantGadget::alloc_const(
            cs.ns(|| "max non signers"),
            &SW6Fr::from(self.max_non_signers),
        )?;

        // We're later on comparing the public input to the sum of public keys in the last block.
        // Hence, we start with the sum of all genesis keys.
        let mut sum = Cow::Borrowed(&generator_var);
        for (i, key) in genesis_keys_var.iter().enumerate() {
            sum = Cow::Owned(sum.add(cs.ns(|| format!("add genesis key {}", i)), key)?);
        }
        sum = Cow::Owned(sum.sub(cs.ns(|| "finalize genesis key sum"), &generator_var)?);

        // We always require at least one block to be verified.
        // This block then also provides the initial values for conditional selects during
        // the rest of the circuit. One example is the last block's list of public keys.
        let mut block_number = epoch_length.clone(); // Our first block to verify ist at EPOCH_LENGTH.
        let mut first_block_var = blocks_var.remove(0);
        let mut prev_public_keys = &genesis_keys_var[..];
        let mut prev_public_key_sum = first_block_var.conditional_verify(
            cs.ns(|| "verify block 0"),
            &prev_public_keys,
            &sum,
            &max_non_signers_var,
            &block_number,
            &generator_var,
            &Boolean::constant(true),
        )?;
        prev_public_keys = first_block_var.public_keys();

        // The verification starts with true and is recalculated for each block as
        // `verification_flag & block_flag`, where `block_flag` denotes the bit indicating
        // whether the current block needs to be checked.
        // After this calculation, we enforce `verification_flag == block_flag`.
        // This enforces that once a block flag was set to false, no following block flag might
        // ever be set to true again.
        let mut verification_flag = Boolean::constant(true);
        for (i, (block, block_flag)) in blocks_var
            .iter_mut()
            .zip(block_flags_var.iter())
            .enumerate()
        {
            // TODO: Use Fq instead.
            block_number = UInt32::addmany(
                cs.ns(|| format!("block number for {}", i + 1)),
                &[block_number, epoch_length.clone()],
            )?;

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

            prev_public_key_sum = block.conditional_verify(
                cs.ns(|| format!("verify block {}", i + 1)),
                &prev_public_keys,
                &prev_public_key_sum,
                &max_non_signers_var,
                &block_number,
                &generator_var,
                &block_flag,
            )?;
            prev_public_keys = block.public_keys();
        }

        // Finally verify that the last block's public key sum correspond to the public input.
        last_public_key_sum_var
            .enforce_equal(cs.ns(|| "public key equality"), &prev_public_key_sum)?;

        Ok(())
    }
}
