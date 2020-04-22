use core::cmp::Ordering;
use std::borrow::{Borrow, Cow};

use algebra::mnt6_753::{Fq, FqParameters};
use algebra::{mnt4_753::Fr as MNT4Fr, One};
use crypto_primitives::prf::blake2s::constraints::{
    blake2s_gadget_with_parameters, Blake2sOutputGadget,
};
use crypto_primitives::prf::Blake2sWithParameterBlock;
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::{
    AllocGadget, Boolean, CondSelectGadget, FieldGadget, GroupGadget, UInt32, UInt8,
};
use r1cs_std::{Assignment, ToBitsGadget};

use crate::constants::VALIDATOR_SLOTS;
use crate::gadgets::mnt4::{
    CheckSigGadget, MNT4YToBitGadget, PedersenCommitmentGadget, PedersenHashGadget,
};
use crate::gadgets::y_to_bit::YToBitGadget;
use crate::primitives::MacroBlock;
use crate::utils::{pad_point_bits, reverse_inner_byte_order};

/// A simple enum representing the two rounds of signing in the macro blocks.
#[derive(Clone, Copy, Ord, PartialOrd, PartialEq, Eq)]
pub enum Round {
    Prepare = 0,
    Commit = 1,
}

/// A gadget representing a macro block in Albatross.
pub struct MacroBlockGadget {
    pub header_hash: Vec<Boolean>,
    pub public_keys: Vec<G2Gadget>,
    pub prepare_signature: G1Gadget,
    pub prepare_signer_bitmap: Vec<Boolean>,
    pub commit_signature: G1Gadget,
    pub commit_signer_bitmap: Vec<Boolean>,
}

impl MacroBlockGadget {
    /// A convenience method to return the public keys of the macro block.
    pub fn public_keys(&self) -> &[G2Gadget] {
        &self.public_keys
    }

    /// A function that verifies the validity of a given macro block. It is the main function for
    /// the macro block gadget.
    pub fn verify<CS: ConstraintSystem<MNT4Fr>>(
        &self,
        mut cs: CS,
        // This is the set of public keys that signed this macro block. Corresponds to the previous
        // set of validators.
        prev_public_keys: &[G2Gadget],
        // This is the maximum number of non-signers for the block. It is inclusive, meaning that if
        // the number of non-signers == max_non-signers then the block is still valid.
        max_non_signers: &FqGadget,
        // Simply the number of the macro block.
        block_number: &UInt32,
        // The generator used in the BLS signature scheme. It is the generator used to create public
        // keys.
        sig_generator: &G2Gadget,
        // The two next generators are only needed because the elliptic curve addition in-circuit is
        // incomplete. Meaning that it can't handle the identity element (aka zero, aka point-at-infinity).
        // So, these generators are needed to do running sums. Instead of starting at zero, we start
        // with the generator and subtract it at the end of the running sum.
        sum_generator_g1: &G1Gadget,
        sum_generator_g2: &G2Gadget,
        // These are just the parameters for the Pedersen hash gadget.
        pedersen_generators: &Vec<G1Gadget>,
    ) -> Result<(), SynthesisError> {
        // Get the hash point and the aggregated public key for the prepare round of signing.
        let (hash0, pub_key0) = self.get_hash_and_public_keys(
            cs.ns(|| "prepare"),
            Round::Prepare,
            prev_public_keys,
            max_non_signers,
            block_number,
            sum_generator_g1,
            sum_generator_g2,
            pedersen_generators,
        )?;

        // Get the hash point and the aggregated public key for the commit round of signing.
        let (hash1, pub_key1) = self.get_hash_and_public_keys(
            cs.ns(|| "commit"),
            Round::Commit,
            prev_public_keys,
            max_non_signers,
            block_number,
            sum_generator_g1,
            sum_generator_g2,
            pedersen_generators,
        )?;

        // Add together the two aggregated signatures for the prepare and commit rounds of signing.
        // Note the use of the generator to avoid an error in the sum.
        let mut signature =
            sum_generator_g1.add(cs.ns(|| "add prepare sig"), &self.prepare_signature)?;
        signature = signature.add(cs.ns(|| "add commit sig"), &self.commit_signature)?;
        signature = signature.sub(cs.ns(|| "finalize sig"), sum_generator_g1)?;

        // Verifies the validity of the signatures.
        CheckSigGadget::check_signatures(
            cs.ns(|| "check signatures"),
            &[pub_key0, pub_key1],
            sig_generator,
            &signature,
            &[hash0, hash1],
        )?;

        Ok(())
    }

    /// A function that returns the aggregated public key and the hash point, for a given round,
    /// of the macro block.
    fn get_hash_and_public_keys<CS: ConstraintSystem<MNT4Fr>>(
        &self,
        mut cs: CS,
        round: Round,
        prev_public_keys: &[G2Gadget],
        max_non_signers: &FqGadget,
        block_number: &UInt32,
        sum_generator_g1: &G1Gadget,
        sum_generator_g2: &G2Gadget,
        pedersen_generators: &Vec<G1Gadget>,
    ) -> Result<(G1Gadget, G2Gadget), SynthesisError> {
        // Calculate the Pedersen hash for the given macro block and round.
        let hash_point = self.hash(
            cs.ns(|| "create hash point"),
            round,
            block_number,
            sum_generator_g1,
            pedersen_generators,
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

        // Calculate the an aggregate public key given the set of public keys and a bitmap of signers.
        let aggregate_public_key = Self::aggregate_public_key(
            cs.ns(|| "aggregate public keys"),
            prev_public_keys,
            signer_bitmap,
            reference_bitmap,
            max_non_signers,
            sum_generator_g2,
        )?;

        Ok((hash_point, aggregate_public_key))
    }

    /// A function that calculates the hash for the block from:
    /// round number || block number || header_hash || public_keys
    /// where || means concatenation.
    /// First we use the Pedersen commitment to compress the input. Then we serialize the resulting
    /// EC point and hash it with the Blake2s hash algorithm, getting an output of 256 bits. This is
    /// necessary because the Pedersen commitment is not pseudo-random and we need pseudo-randomness
    /// for the BLS signature scheme. Finally we use the Pedersen hash algorithm on those 256 bits
    /// to obtain a single EC point.
    pub fn hash<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        &self,
        mut cs: CS,
        round: Round,
        block_number: &UInt32,
        sum_generator_g1: &G1Gadget,
        pedersen_generators: &Vec<G1Gadget>,
    ) -> Result<G1Gadget, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits: Vec<Boolean> = vec![];

        // The round number comes in little endian,
        // which is why we need to reverse the bits to get big endian.
        let round_number = UInt8::constant(round as u8);
        let mut round_number_bits = round_number.into_bits_le();
        round_number_bits.reverse();
        bits.append(&mut round_number_bits);

        // The block number comes in little endian all the way.
        // So, a reverse will put it into big endian.
        let mut block_number_be = block_number.to_bits_le();
        block_number_be.reverse();
        bits.append(&mut block_number_be);

        // Append the header hash.
        bits.extend_from_slice(&self.header_hash);

        // Serialize all the public keys.
        for i in 0..self.public_keys.len() {
            let key = &self.public_keys[i];
            // Get bits from the x coordinate.
            let x_bits: Vec<Boolean> = key.x.to_bits(cs.ns(|| format!("x to bits: pk {}", i)))?;
            // Get one bit from the y coordinate.
            let greatest_bit =
                MNT4YToBitGadget::y_to_bit_g2(cs.ns(|| format!("y to bits: pk {}", i)), key)?;
            // Pad points and get *Big-Endian* representation.
            let serialized_bits = pad_point_bits::<FqParameters>(x_bits, greatest_bit);
            // Append to Boolean vector.
            bits.extend(serialized_bits);
        }

        // Calculate the Pedersen commitment.
        let pedersen_commitment = PedersenCommitmentGadget::evaluate(
            cs.ns(|| "pedersen commitment"),
            pedersen_generators,
            &bits,
            &sum_generator_g1,
        )?;

        // Serialize the Pedersen commitment.
        let x_bits = pedersen_commitment
            .x
            .to_bits(cs.ns(|| "x to bits: pedersen commitment"))?;
        let greatest_bit = MNT4YToBitGadget::y_to_bit_g1(
            cs.ns(|| "y to bit: pedersen commitment"),
            &pedersen_commitment,
        )?;
        let serialized_bits = pad_point_bits::<FqParameters>(x_bits, greatest_bit);

        // Prepare order of booleans for blake2s (it doesn't expect Big-Endian).
        let serialized_bits = reverse_inner_byte_order(&serialized_bits);

        // Initialize Blake2s parameters.
        let blake2s_parameters = Blake2sWithParameterBlock {
            digest_length: 32,
            key_length: 0,
            fan_out: 1,
            depth: 1,
            leaf_length: 0,
            node_offset: 0,
            xof_digest_length: 0,
            node_depth: 0,
            inner_length: 0,
            salt: [0; 8],
            personalization: [0; 8],
        };

        // Calculate Blake2s hash.
        let hash = blake2s_gadget_with_parameters(
            cs.ns(|| "blake2s hash from serialized bits"),
            &serialized_bits,
            &blake2s_parameters.parameters(),
        )?;

        // Convert to bits.
        let mut result = Vec::new();
        for i in 0..8 {
            result.extend(hash[i].to_bits_le());
        }

        // Finally feed the bits into the Pedersen hash gadget.
        let pedersen_result = PedersenHashGadget::evaluate(
            &mut cs.ns(|| "crh_evaluation"),
            pedersen_generators,
            &result,
            sum_generator_g1,
        )?;

        Ok(pedersen_result)
    }

    /// A function that aggregates the public keys of all the validators that signed the block. If
    /// it is performing the aggregation for the commit round, then it will also check if every signer
    /// in the commit round was also a signer in the prepare round.
    pub fn aggregate_public_key<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        public_keys: &[G2Gadget],
        key_bitmap: &[Boolean],
        reference_bitmap: Option<&[Boolean]>,
        max_non_signers: &FqGadget,
        sum_generator_g2: &G2Gadget,
    ) -> Result<G2Gadget, SynthesisError> {
        // Initialize the running sums.
        // Note that we initialize the public key sum to the generator, not to zero.
        let mut num_non_signers = FqGadget::zero(cs.ns(|| "number used public keys"))?;
        let mut sum = Cow::Borrowed(sum_generator_g2);

        // Conditionally add all other public keys.
        for (i, (key, included)) in public_keys.iter().zip(key_bitmap.iter()).enumerate() {
            // Calculate a new sum that includes the next public key.
            let new_sum = sum.add(cs.ns(|| format!("add public key {}", i)), key)?;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally add public key {}", i)),
                included,
                &new_sum,
                sum.as_ref(),
            )?;

            // If there is a reference bitmap, we only count such signatures as included
            // that fulfill included & reference[i]. That means, we only count these that
            // also signed in the reference bitmap (usually the prepare phase).
            let included = if let Some(reference) = reference_bitmap {
                Boolean::and(
                    cs.ns(|| format!("included & reference[{}]", i)),
                    included,
                    &reference[i],
                )?
            } else {
                *included
            };

            // Update the number of non-signers. Note that the bitmap is negated to get the
            // non-signers: ~(included).
            num_non_signers = num_non_signers.conditionally_add_constant(
                cs.ns(|| format!("public key count {}", i)),
                &included.not(),
                Fq::one(),
            )?;

            // Update the public key sum.
            sum = Cow::Owned(cond_sum);
        }

        // Finally subtract the generator from the sum to get the correct value.
        sum = Cow::Owned(sum.sub(cs.ns(|| "finalize aggregate public key"), sum_generator_g2)?);

        // Enforce that there are enough signers. Specifically that:
        // num_non_signers <= max_non_signers
        num_non_signers.enforce_cmp(
            cs.ns(|| "enforce non signers"),
            max_non_signers,
            Ordering::Less,
            true,
        )?;

        Ok(sum.into_owned())
    }
}

/// The allocation function for the macro block gadget.
impl AllocGadget<MacroBlock, MNT4Fr> for MacroBlockGadget {
    /// This is the allocation function for a constant. It does not need to be implemented.
    fn alloc_constant<T, CS: ConstraintSystem<MNT4Fr>>(
        _cs: CS,
        _val: T,
    ) -> Result<Self, SynthesisError>
    where
        T: Borrow<MacroBlock>,
    {
        unimplemented!()
    }

    /// This is the allocation function for a private input.
    fn alloc<F, T, CS: ConstraintSystem<MNT4Fr>>(
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

        assert_eq!(value.public_keys.len(), VALIDATOR_SLOTS);

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
        })
    }

    /// This is the allocation function for a public input.
    fn alloc_input<F, T, CS: ConstraintSystem<MNT4Fr>>(
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

        assert_eq!(value.public_keys.len(), VALIDATOR_SLOTS);

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
        })
    }
}
