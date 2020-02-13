use std::marker::PhantomData;
use std::ops::Add;

use crate::gadgets::macro_block::{MacroBlock, MacroBlockGadget};
use algebra::curves::bls12_377::{Bls12_377, Bls12_377Parameters, G1Projective, G2Projective};
use algebra::fields::bls12_377::fq::Fq;
use algebra::fields::sw6::Fr as SW6Fr;
use algebra::{Field, PrimeField};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, LinearCombination, SynthesisError};
use r1cs_std::bits::boolean::AllocatedBit;
use r1cs_std::eq::{ConditionalEqGadget, EqGadget};
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::{
    G1Gadget, G1PreparedGadget, G2Gadget, G2PreparedGadget,
};
use r1cs_std::pairing::bls12_377::PairingGadget;
use r1cs_std::pairing::PairingGadget as PG;
use r1cs_std::prelude::*;
use std::str::FromStr;

pub struct Benchmark {
    genesis_keys: Vec<G2Projective>,
    signer_bitmap: Vec<bool>,
    test_block: MacroBlock,
    signature: G1Projective,
    generator: G2Projective,
    max_non_signers: u64,
}

impl Benchmark {
    pub fn new(
        genesis_keys: Vec<G2Projective>,
        signer_bitmap: Vec<bool>,
        test_block: MacroBlock,
        signature: G1Projective,
        generator: G2Projective,
        max_non_signers: u64,
    ) -> Self {
        Self {
            genesis_keys,
            signer_bitmap,
            test_block,
            generator,
            signature,
            max_non_signers,
        }
    }
}

impl ConstraintSynthesizer<Fq> for Benchmark {
    fn generate_constraints<CS: ConstraintSystem<Fq>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        let mut genesis_keys_var = vec![];
        // TODO: let input = Vec::<Boolean>::alloc(cs.ns(|| "Input"), || Ok(scalar)).unwrap();
        for (i, key) in self.genesis_keys.iter().enumerate() {
            let key_var = G2Gadget::<Bls12_377Parameters>::alloc(
                cs.ns(|| format!("genesis key {}", i)),
                || Ok(key),
            )?;
            genesis_keys_var.push(key_var);
        }

        let mut signer_bitmap_var = vec![];
        for (i, signer) in self.signer_bitmap.iter().enumerate() {
            let signed = Boolean::from(AllocatedBit::alloc(
                cs.ns(|| format!("signer bitmap {}", i)),
                || Ok(signer),
            )?);
            signer_bitmap_var.push(signed);
        }

        let block_var =
            MacroBlockGadget::alloc(cs.ns(|| "first macro block"), || Ok(&self.test_block))?;

        let signature_var =
            G1Gadget::<Bls12_377Parameters>::alloc(cs.ns(|| "signature"), || Ok(&self.signature))?;
        let generator_var =
            G2Gadget::<Bls12_377Parameters>::alloc(cs.ns(|| "sig"), || Ok(&self.generator))?;

        let mut max_non_signers_var = FpGadget::zero(cs.ns(|| "max_signers"))?;
        max_non_signers_var.add_constant_in_place(
            cs.ns(|| "add max_non_signers"),
            &SW6Fr::from(self.max_non_signers),
        )?;

        block_var.verify(
            cs.ns(|| "verify block"),
            &genesis_keys_var,
            &signer_bitmap_var,
            &max_non_signers_var,
            &signature_var,
            &generator_var,
        )?;

        Ok(())
    }
}
