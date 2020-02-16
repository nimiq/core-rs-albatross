use crate::gadgets::check_sig::CheckSigGadget;
use crate::gadgets::smaller_than::SmallerThanGadget;
use crate::gadgets::y_to_bit::YToBitGadget;
use crate::gadgets::{pad_point_bits, reverse_inner_byte_order};
use crate::macro_block::MacroBlock;
use crate::setup::CRH;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};
use algebra::curves::bls12_377::{Bls12_377Parameters, G1Projective};
use algebra::fields::bls12_377::FqParameters;
use algebra::fields::sw6::Fr as SW6Fr;
use algebra::One;
use crypto_primitives::crh::pedersen::constraints::PedersenCRHGadget;
use crypto_primitives::prf::blake2s::constraints::Blake2sOutputGadget;
use crypto_primitives::FixedLengthCRHGadget;
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

pub type CRHGadget = PedersenCRHGadget<G1Projective, SW6Fr, G1Gadget>;
pub type CRHGadgetParameters = <CRHGadget as FixedLengthCRHGadget<CRH, SW6Fr>>::ParametersGadget;

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
        sum_generator_g1: &G1Gadget,
        sum_generator_g2: &G2Gadget,
        crh_parameters: &CRHGadgetParameters,
        condition: &Boolean,
    ) -> Result<G2Gadget, SynthesisError> {
        // Verify prepare signature.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Verify signatures");
        let (hash0, pub_key0) = self.get_hash_and_public_keys(
            cs.ns(|| "prepare"),
            Round::Prepare,
            prev_public_keys,
            max_non_signers,
            block_number,
            sum_generator_g2,
            crh_parameters,
        )?;

        // Verify commit signature.
        next_cost_analysis!(cs, cost, || "Verify commit signature");
        let (hash1, pub_key1) = self.get_hash_and_public_keys(
            cs.ns(|| "commit"),
            Round::Commit,
            prev_public_keys,
            max_non_signers,
            block_number,
            sum_generator_g2,
            crh_parameters,
        )?;

        next_cost_analysis!(cs, cost, || "Add signatures");
        let mut signature =
            sum_generator_g1.add(cs.ns(|| "add prepare sig"), &self.prepare_signature)?;
        signature = signature.add(cs.ns(|| "add commit sig"), &self.commit_signature)?;
        signature = signature.sub(cs.ns(|| "finalize sig"), sum_generator_g1)?;

        next_cost_analysis!(cs, cost, || "Check signature");
        CheckSigGadget::conditional_check_signatures(
            cs.ns(|| "check signatures"),
            &[pub_key0, pub_key1],
            generator,
            &signature,
            &[hash0, hash1],
            condition,
        )?;

        // Either return the prev_public_key_sum or this block's public key sum,
        // depending on the condition.
        // If condition is true, select current key, else previous key.
        next_cost_analysis!(cs, cost, || "Select public key");
        let last_verified_public_key_sum = CondSelectGadget::conditionally_select(
            cs.ns(|| "select pubkey"),
            condition,
            self.sum_public_keys.as_ref().get()?,
            prev_public_key_sum,
        )?;
        end_cost_analysis!(cs, cost);
        Ok(last_verified_public_key_sum)
    }

    fn get_hash_and_public_keys<CS: ConstraintSystem<SW6Fr>>(
        &mut self,
        mut cs: CS,
        round: Round,
        prev_public_keys: &[G2Gadget],
        max_non_signers: &FpGadget<SW6Fr>,
        block_number: &UInt32,
        generator: &G2Gadget,
        crh_parameters: &CRHGadgetParameters,
    ) -> Result<(G1Gadget, G2Gadget), SynthesisError> {
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Create hash point");
        let hash_point = self.to_g1(
            cs.ns(|| "create hash point"),
            round,
            block_number,
            generator,
            crh_parameters,
        )?;

        // Choose signer bitmap and signature based on round.
        let signer_bitmap = match round {
            Round::Prepare => &self.prepare_signer_bitmap,
            Round::Commit => &self.commit_signer_bitmap,
        };

        // Also, during the commit round, we need to check the max-non-signer restriction
        // with respect to prepare_signer_bitmap & commit_signer_bitmap.
        let reference_bitmap = match round {
            Round::Prepare => None,
            Round::Commit => Some(self.prepare_signer_bitmap.as_ref()),
        };

        next_cost_analysis!(cs, cost, || "Aggregate public key");
        let aggregate_public_key = Self::aggregate_public_key(
            cs.ns(|| "aggregate public keys"),
            prev_public_keys,
            signer_bitmap,
            reference_bitmap,
            max_non_signers,
            generator,
        )?;
        end_cost_analysis!(cs, cost);
        Ok((hash_point, aggregate_public_key))
    }

    fn to_g1<CS: ConstraintSystem<SW6Fr>>(
        &mut self,
        mut cs: CS,
        round: Round,
        block_number: &UInt32,
        generator: &G2Gadget,
        crh_parameters: &CRHGadgetParameters,
    ) -> Result<G1Gadget, SynthesisError> {
        // Sum public keys on first call.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Sum public keys");
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

        next_cost_analysis!(cs, cost, || "Construct Pedersen Hash");
        let g1 = self.hash(
            cs.ns(|| "prefix || header_hash || sum_pks to hash"),
            &round_number,
            block_number,
            sum,
            crh_parameters,
        )?;
        end_cost_analysis!(cs, cost);
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
            cs.ns(|| "enforce non signers"),
            &num_non_signers,
            max_non_signers,
        )?;

        Ok(sum.into_owned())
    }

    /// Calculates the Pedersen Hash for the block from:
    /// prefix || header_hash || sum_pks
    ///
    /// Note that the Pedersen Hash is only collision-resistant
    /// and does not provide pseudo-random output!
    /// For our use-case, however, this suffices as the `header_hash`
    /// provides enough entropy.
    pub fn hash<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        &self,
        mut cs: CS,
        round_number: &UInt8,
        block_number: &UInt32,
        g2: &G2Gadget,
        crh_parameters: &CRHGadgetParameters,
    ) -> Result<G1Gadget, SynthesisError> {
        // Convert g2 to bits before hashing.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Convert g2 to bits");
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
        let input_bytes: Vec<UInt8> = bits
            .chunks(8)
            .map(|chunk| UInt8::from_bits_le(chunk))
            .collect();

        // Hash serialized bits.
        next_cost_analysis!(cs, cost, || "Pedersen Hash");
        let crh_result = <CRHGadget as FixedLengthCRHGadget<CRH, SW6Fr>>::check_evaluation_gadget(
            &mut cs.ns(|| "crh_evaluation"),
            crh_parameters,
            &input_bytes,
        )?;
        end_cost_analysis!(cs, cost);
        Ok(crh_result)
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
