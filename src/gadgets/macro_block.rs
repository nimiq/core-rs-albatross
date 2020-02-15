use crate::gadgets::check_sig::CheckSigGadget;
use crate::gadgets::smaller_than::SmallerThanGadget;
use crate::gadgets::xof_hash::XofHashGadget;
use crate::gadgets::xof_hash_to_g1::XofHashToG1Gadget;
use crate::gadgets::y_to_bit::YToBitGadget;
use crate::gadgets::{hash_to_bits, pad_point_bits, reverse_inner_byte_order};
use algebra::curves::bls12_377::Bls12_377Parameters;
use algebra::curves::bls12_377::{G1Projective, G2Projective};
use algebra::fields::bls12_377::FqParameters;
use algebra::fields::bls12_377::Fr;
use algebra::fields::sw6::Fr as SW6Fr;
use algebra::{One, ProjectiveCurve, Zero};
use crypto_primitives::prf::blake2s::constraints::blake2s_gadget;
use crypto_primitives::prf::blake2s::constraints::Blake2sOutputGadget;
use nimiq_bls::PublicKey;
use nimiq_hash::{Blake2sHash, Blake2sHasher, Hasher};
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
use std::ops::Mul;

#[derive(Clone)]
pub struct MacroBlock {
    pub header_hash: [u8; 32],
    pub public_keys: Vec<G2Projective>,
    pub signature: Option<G1Projective>,
    pub signer_bitmap: Vec<bool>,
}

impl MacroBlock {
    pub const SLOTS: usize = 2; // TODO: Set correctly.

    pub fn hash(&self, round_number: u8, block_number: u32) -> Blake2sHash {
        let mut sum = G2Projective::zero();
        for key in self.public_keys.iter() {
            sum += key;
        }

        let sum_key = PublicKey { public_key: sum };

        // Then, concatenate the prefix || header_hash || sum_pks,
        // with prefix = (round number || block number).
        let mut prefix = vec![round_number];
        prefix.extend_from_slice(&block_number.to_be_bytes());
        let mut msg = prefix;
        msg.extend_from_slice(&self.header_hash);
        msg.extend_from_slice(sum_key.compress().as_ref());

        Blake2sHasher::new().chain(&msg).finish()
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            header_hash: [0; 32],
            // TODO: Replace by a proper number.
            public_keys: vec![
                G2Projective::prime_subgroup_generator().mul(&Fr::from(2149u32));
                MacroBlock::SLOTS
            ],
            signature: Some(G1Projective::prime_subgroup_generator().mul(&Fr::from(2013u32))),
            signer_bitmap: vec![true; MacroBlock::SLOTS],
        }
    }
}

pub struct MacroBlockGadget {
    pub header_hash: Vec<Boolean>,
    pub public_keys: Vec<G2Gadget>,
    pub signature: G1Gadget,
    pub signer_bitmap: Vec<Boolean>,
}

impl MacroBlockGadget {
    pub const MAXIMUM_NON_SIGNERS: u64 = 170;

    pub fn public_keys(&self) -> &[G2Gadget] {
        &self.public_keys
    }

    pub fn conditional_verify<CS: ConstraintSystem<SW6Fr>>(
        &self,
        mut cs: CS,
        prev_public_keys: &[G2Gadget],
        max_non_signers: &FpGadget<SW6Fr>,
        block_number: &UInt32,
        generator: &G2Gadget,
        condition: &Boolean,
    ) -> Result<Vec<G2Gadget>, SynthesisError> {
        let hash_point = self.to_g1(
            cs.ns(|| "create hash point"),
            &UInt8::constant(0),
            block_number,
            generator,
        )?;

        let aggregate_public_key = Self::aggregate_public_key(
            cs.ns(|| "aggregate public keys"),
            prev_public_keys,
            &self.signer_bitmap,
            max_non_signers,
            generator,
        )?;

        CheckSigGadget::conditional_check_signature(
            cs.ns(|| "check signature"),
            &aggregate_public_key,
            generator,
            &self.signature,
            &hash_point,
            condition,
        )?;

        // Either return the prev_public_keys or this block's public keys,
        // depending on the condition.
        let mut public_keys = vec![];
        for (i, (prev_key, current_key)) in prev_public_keys
            .iter()
            .zip(self.public_keys.iter())
            .enumerate()
        {
            // If condition is true, select current key, else previous key.
            let last_verified_public_key = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("select pubkey {}", i)),
                condition,
                current_key,
                prev_key,
            )?;
            public_keys.push(last_verified_public_key);
        }
        Ok(public_keys)
    }

    pub fn to_g1<CS: ConstraintSystem<SW6Fr>>(
        &self,
        mut cs: CS,
        round_number: &UInt8,
        block_number: &UInt32,
        generator: &G2Gadget,
    ) -> Result<G1Gadget, SynthesisError> {
        // Sum public keys.
        let mut sum = Cow::Borrowed(generator);
        for (i, key) in self.public_keys.iter().enumerate() {
            sum = Cow::Owned(sum.add(cs.ns(|| format!("add public key {}", i)), key)?);
        }
        sum = Cow::Owned(sum.sub(cs.ns(|| "finalize sum"), generator)?);

        let hash = self.hash(
            cs.ns(|| "prefix || header_hash || sum_pks to hash"),
            round_number,
            block_number,
            &sum,
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
        let signer_bitmap =
            Vec::<Boolean>::alloc(cs.ns(|| "signer bitmap"), || Ok(&value.signer_bitmap[..]))?;
        let signature = G1Gadget::alloc(cs.ns(|| "signature"), || value.signature.get())?;

        Ok(MacroBlockGadget {
            header_hash,
            public_keys,
            signature,
            signer_bitmap,
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
        let signer_bitmap = Vec::<Boolean>::alloc_input(cs.ns(|| "signer bitmap"), || {
            Ok(&value.signer_bitmap[..])
        })?;
        let signature = G1Gadget::alloc_input(cs.ns(|| "signature"), || value.signature.get())?;

        Ok(MacroBlockGadget {
            header_hash,
            public_keys,
            signature,
            signer_bitmap,
        })
    }
}
