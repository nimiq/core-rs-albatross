use crate::gadgets::check_sig::CheckSigGadget;
use crate::gadgets::smaller_than::SmallerThanGadget;
use crate::gadgets::xof_hash::XofHashGadget;
use crate::gadgets::xof_hash_to_g1::XofHashToG1Gadget;
use crate::gadgets::y_to_bit::YToBitGadget;
use crate::gadgets::{hash_to_bits, pad_point_bits, reverse_inner_byte_order};
use crate::macro_block::MacroBlock;
use algebra::curves::bls12_377::Bls12_377Parameters;
use algebra::fields::bls12_377::FqParameters;
use algebra::fields::sw6::Fr as SW6Fr;
use algebra::One;
use crypto_primitives::prf::blake2s::constraints::blake2s_gadget;
use crypto_primitives::prf::blake2s::constraints::Blake2sOutputGadget;
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::bits::uint32::UInt32;
use r1cs_std::bits::uint8::UInt8;
use r1cs_std::bits::ToBitsGadget;
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::bls12_377::{G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, CondSelectGadget, FieldGadget, GroupGadget};
use r1cs_std::Assignment;
use std::borrow::{Borrow, Cow};

#[derive(Clone, Copy, Ord, PartialOrd, PartialEq, Eq)]
enum Round {
    Prepare = 0,
    Commit = 1,
}

pub struct MacroBlockGadget {
    pub header_hash: Vec<Boolean>,
    pub public_keys: Vec<G2Gadget>,
    pub prepare_signature: G1Gadget,
    pub prepare_signer_bitmap: Vec<Boolean>,
    pub commit_signature: G1Gadget,
    pub commit_signer_bitmap: Vec<Boolean>,

    // Internal state used for caching.
    sum_public_keys: Option<G2Gadget>,
}

impl MacroBlockGadget {
    pub const MAXIMUM_NON_SIGNERS: u64 = 170;

    pub fn public_keys(&self) -> &[G2Gadget] {
        &self.public_keys
    }

    pub fn conditional_verify<CS: ConstraintSystem<SW6Fr>>(
        &mut self,
        mut cs: CS,
        prev_public_keys: &[G2Gadget],
        prev_public_key_sum: &G2Gadget,
        max_non_signers: &FpGadget<SW6Fr>,
        block_number: &UInt32,
        generator: &G2Gadget,
        condition: &Boolean,
    ) -> Result<G2Gadget, SynthesisError> {
        // Verify prepare signature.
        self.conditional_verify_signature(
            cs.ns(|| "prepare"),
            Round::Prepare,
            prev_public_keys,
            max_non_signers,
            block_number,
            generator,
            condition,
        )?;

        // Verify commit signature.
        self.conditional_verify_signature(
            cs.ns(|| "commit"),
            Round::Commit,
            prev_public_keys,
            max_non_signers,
            block_number,
            generator,
            condition,
        )?;

        // Either return the prev_public_key_sum or this block's public key sum,
        // depending on the condition.
        // If condition is true, select current key, else previous key.
        let last_verified_public_key_sum = CondSelectGadget::conditionally_select(
            cs.ns(|| "select pubkey"),
            condition,
            self.sum_public_keys.as_ref().get()?,
            prev_public_key_sum,
        )?;
        Ok(last_verified_public_key_sum)
    }

    fn conditional_verify_signature<CS: ConstraintSystem<SW6Fr>>(
        &mut self,
        mut cs: CS,
        round: Round,
        prev_public_keys: &[G2Gadget],
        max_non_signers: &FpGadget<SW6Fr>,
        block_number: &UInt32,
        generator: &G2Gadget,
        condition: &Boolean,
    ) -> Result<(), SynthesisError> {
        let hash_point = self.to_g1(
            cs.ns(|| "create hash point"),
            round,
            block_number,
            generator,
        )?;

        // Choose signer bitmap and signature based on round.
        let (signer_bitmap, signature) = match round {
            Round::Prepare => (&self.prepare_signer_bitmap, &self.prepare_signature),
            Round::Commit => (&self.commit_signer_bitmap, &self.commit_signature),
        };

        // Also, during the commit round, we need to check the max-non-signer restriction
        // with respect to prepare_signer_bitmap & commit_signer_bitmap.
        let reference_bitmap = match round {
            Round::Prepare => None,
            Round::Commit => Some(self.prepare_signer_bitmap.as_ref()),
        };

        let aggregate_public_key = Self::aggregate_public_key(
            cs.ns(|| "aggregate public keys"),
            prev_public_keys,
            signer_bitmap,
            reference_bitmap,
            max_non_signers,
            generator,
        )?;

        CheckSigGadget::conditional_check_signature(
            cs.ns(|| "check signature"),
            &aggregate_public_key,
            generator,
            signature,
            &hash_point,
            condition,
        )?;
        Ok(())
    }

    fn to_g1<CS: ConstraintSystem<SW6Fr>>(
        &mut self,
        mut cs: CS,
        round: Round,
        block_number: &UInt32,
        generator: &G2Gadget,
    ) -> Result<G1Gadget, SynthesisError> {
        // Sum public keys on first call.
        if self.sum_public_keys.is_none() {
            let mut sum = Cow::Borrowed(generator);
            for (i, key) in self.public_keys.iter().enumerate() {
                sum = Cow::Owned(sum.add(cs.ns(|| format!("add public key {}", i)), key)?);
            }
            sum = Cow::Owned(sum.sub(cs.ns(|| "finalize sum"), generator)?);

            self.sum_public_keys = Some(sum.into_owned());
        }
        // Then use value from circuit.
        let sum = self.sum_public_keys.as_ref().unwrap();

        let round_number = UInt8::constant(round as u8);

        let hash = self.hash(
            cs.ns(|| "prefix || header_hash || sum_pks to hash"),
            &round_number,
            block_number,
            sum,
        )?;
        assert_eq!(hash.len(), 32 * 8);

        // Feed normal blake2s hash into XOF.
        // Our gadget expects normal *Big-Endian* order.
        let xof_bits = XofHashGadget::xof_hash(cs.ns(|| "xof hash"), &hash)?;

        // Convert to G1 using try-and-increment method.
        let g1 = XofHashToG1Gadget::hash_to_g1(cs.ns(|| "xor hash to g1"), &xof_bits)?;

        Ok(g1)
    }

    pub fn aggregate_public_key<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        public_keys: &[G2Gadget],
        key_bitmap: &[Boolean],
        reference_bitmap: Option<&[Boolean]>,
        max_non_signers: &FpGadget<SW6Fr>,
        generator: &G2Gadget,
    ) -> Result<G2Gadget, SynthesisError> {
        // Sum public keys.
        let mut num_non_signers = FpGadget::zero(cs.ns(|| "num used public keys"))?;

        let mut sum = Cow::Borrowed(generator);

        // Conditionally add all other public keys.
        for (i, (key, included)) in public_keys.iter().zip(key_bitmap.iter()).enumerate() {
            let new_sum = sum.add(cs.ns(|| format!("add public key {}", i)), key)?;
            let cond_sum = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally add public key {}", i)),
                included,
                &new_sum,
                sum.as_ref(),
            )?;

            // If there is a reference bitmap, we only count such signatures as included
            // that fulfill included & reference[i]. That means, we only count these that
            // also signed in the reference bitmap (usually the prepare phase).
            // The result is then negated to get the non-signers: ~(included & reference[i]).
            let included = if let Some(reference) = reference_bitmap {
                Boolean::and(
                    cs.ns(|| format!("included & reference[{}]", i)),
                    included,
                    &reference[i],
                )?
            } else {
                *included
            };
            num_non_signers = num_non_signers.conditionally_add_constant(
                cs.ns(|| format!("public key count {}", i)),
                &included.not(),
                SW6Fr::one(),
            )?;
            sum = Cow::Owned(cond_sum);
        }
        sum = Cow::Owned(sum.sub(cs.ns(|| "finalize aggregate public key"), generator)?);

        // Enforce enough signers.
        SmallerThanGadget::enforce_smaller_than(
            cs.ns(|| "non signers < 171"),
            &num_non_signers,
            max_non_signers,
        )?;

        Ok(sum.into_owned())
    }

    pub fn hash<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        &self,
        mut cs: CS,
        round_number: &UInt8,
        block_number: &UInt32,
        g2: &G2Gadget,
    ) -> Result<Vec<Boolean>, SynthesisError> {
        // Convert g2 to bits before hashing.
        let serialized_bits: Vec<Boolean> = g2.x.to_bits(cs.ns(|| "bits"))?;
        let greatest_bit =
            YToBitGadget::<Bls12_377Parameters>::y_to_bit_g2(cs.ns(|| "y to bit"), g2)?;

        // Pad points and get *Big-Endian* representation.
        let mut serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);

        // Then, concatenate the prefix || header_hash || sum_pks,
        // with prefix = (round number || block number).
        // The round number comes in little endian,
        // which is why we need to reverse the bits to get big endian.
        let mut round_number_bits = round_number.into_bits_le();
        round_number_bits.reverse();
        // The block number comes in little endian all the way.
        // So, a reverse will put it into big endian.
        let mut block_number_be = block_number.to_bits_le();
        block_number_be.reverse();

        // Append everything.
        let mut bits = round_number_bits;
        bits.append(&mut block_number_be);
        bits.extend_from_slice(&self.header_hash);
        bits.append(&mut serialized_bits);

        // Prepare order of booleans for blake2s (it doesn't expect Big-Endian).
        let bits = reverse_inner_byte_order(&bits);

        // Hash serialized bits.
        let h0 = blake2s_gadget(cs.ns(|| "h0 from serialized bits"), &bits)?;
        let h0_bits = hash_to_bits(h0);
        Ok(h0_bits)
    }
}

impl AllocGadget<MacroBlock, SW6Fr> for MacroBlockGadget {
    fn alloc<F, T, CS: ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        value_gen: F,
    ) -> Result<Self, SynthesisError>
    where
        F: FnOnce() -> Result<T, SynthesisError>,
        T: Borrow<MacroBlock>,
    {
        let empty_block = MacroBlock::default();
        let value = match value_gen() {
            Ok(val) => val.borrow().clone(),
            Err(_) => empty_block,
        };

        assert_eq!(value.public_keys.len(), MacroBlock::SLOTS);

        let header_hash =
            Blake2sOutputGadget::alloc(cs.ns(|| "header hash"), || Ok(&value.header_hash))?;
        // While the bytes of the Blake2sOutputGadget start with the most significant first,
        // the bits internally start with the least significant.
        // Thus, we need to reverse the bit order there.
        let header_hash = header_hash
            .0
            .into_iter()
            .flat_map(|n| reverse_inner_byte_order(&n.into_bits_le()))
            .collect::<Vec<Boolean>>();
        let public_keys =
            Vec::<G2Gadget>::alloc(cs.ns(|| "public keys"), || Ok(&value.public_keys[..]))?;

        let prepare_signer_bitmap =
            Vec::<Boolean>::alloc(cs.ns(|| "prepare signer bitmap"), || {
                Ok(&value.prepare_signer_bitmap[..])
            })?;
        let prepare_signature = G1Gadget::alloc(cs.ns(|| "prepare signature"), || {
            value.prepare_signature.get()
        })?;

        let commit_signer_bitmap = Vec::<Boolean>::alloc(cs.ns(|| "commit signer bitmap"), || {
            Ok(&value.commit_signer_bitmap[..])
        })?;
        let commit_signature = G1Gadget::alloc(cs.ns(|| "commit signature"), || {
            value.commit_signature.get()
        })?;

        Ok(MacroBlockGadget {
            header_hash,
            public_keys,
            prepare_signature,
            prepare_signer_bitmap,
            commit_signature,
            commit_signer_bitmap,
            sum_public_keys: None,
        })
    }

    fn alloc_input<F, T, CS: ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        value_gen: F,
    ) -> Result<Self, SynthesisError>
    where
        F: FnOnce() -> Result<T, SynthesisError>,
        T: Borrow<MacroBlock>,
    {
        let empty_block = MacroBlock::default();
        let value = match value_gen() {
            Ok(val) => val.borrow().clone(),
            Err(_) => empty_block,
        };

        assert_eq!(value.public_keys.len(), MacroBlock::SLOTS);

        let header_hash =
            Blake2sOutputGadget::alloc_input(cs.ns(|| "header hash"), || Ok(&value.header_hash))?;
        // While the bytes of the Blake2sOutputGadget start with the most significant first,
        // the bits internally start with the least significant.
        // Thus, we need to reverse the bit order there.
        let header_hash = header_hash
            .0
            .into_iter()
            .flat_map(|n| reverse_inner_byte_order(&n.into_bits_le()))
            .collect::<Vec<Boolean>>();
        let public_keys =
            Vec::<G2Gadget>::alloc_input(cs.ns(|| "public keys"), || Ok(&value.public_keys[..]))?;

        let prepare_signer_bitmap =
            Vec::<Boolean>::alloc_input(cs.ns(|| "prepare signer bitmap"), || {
                Ok(&value.prepare_signer_bitmap[..])
            })?;
        let prepare_signature = G1Gadget::alloc_input(cs.ns(|| "prepare signature"), || {
            value.prepare_signature.get()
        })?;

        let commit_signer_bitmap =
            Vec::<Boolean>::alloc_input(cs.ns(|| "commit signer bitmap"), || {
                Ok(&value.commit_signer_bitmap[..])
            })?;
        let commit_signature = G1Gadget::alloc_input(cs.ns(|| "commit signature"), || {
            value.commit_signature.get()
        })?;

        Ok(MacroBlockGadget {
            header_hash,
            public_keys,
            prepare_signature,
            prepare_signer_bitmap,
            commit_signature,
            commit_signer_bitmap,
            sum_public_keys: None,
        })
    }
}
