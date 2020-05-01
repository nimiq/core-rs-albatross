use core::cmp::Ordering;
use std::borrow::Borrow;

use algebra::mnt6_753::{Fq, FqParameters};
use algebra::{mnt4_753::Fr as MNT4Fr, One};
use crypto_primitives::prf::blake2s::constraints::{
    blake2s_gadget_with_parameters, Blake2sOutputGadget,
};
use crypto_primitives::prf::Blake2sWithParameterBlock;
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, Boolean, FieldGadget, GroupGadget, UInt32, UInt8};
use r1cs_std::{Assignment, ToBitsGadget};

use crate::constants::VALIDATOR_SLOTS;
use crate::gadgets::mnt4::{
    CheckSigGadget, PedersenCommitmentGadget, PedersenHashGadget, YToBitGadget,
};
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
    pub prepare_signature: G1Gadget,
    pub prepare_signer_bitmap: Vec<Boolean>,
    pub commit_signature: G1Gadget,
    pub commit_signer_bitmap: Vec<Boolean>,
}

impl MacroBlockGadget {
    /// A function that verifies the validity of a given macro block. It is the main function for
    /// the macro block gadget.
    pub fn verify<CS: ConstraintSystem<MNT4Fr>>(
        &self,
        mut cs: CS,
        // This is the commitment for the set of public keys that are owned by the next set of validators.
        pks_commitment: &Vec<UInt8>,
        // Simply the number of the macro block.
        block_number: &UInt32,
        // This is the aggregated public key for the prepare round.
        prepare_agg_pk: &G2Gadget,
        // This is the aggregated public key for the commit round.
        commit_agg_pk: &G2Gadget,
        // This is the maximum number of non-signers for the block. It is inclusive, meaning that if
        // the number of non-signers == max_non-signers then the block is still valid.
        max_non_signers: &FqGadget,
        // The generator used in the BLS signature scheme. It is the generator used to create public
        // keys.
        sig_generator: &G2Gadget,
        // The next generator is only needed because the elliptic curve addition in-circuit is
        // incomplete. Meaning that it can't handle the identity element (aka zero, aka point-at-infinity).
        // So, this generator is needed to do running sums. Instead of starting at zero, we start
        // with the generator and subtract it at the end of the running sum.
        sum_generator_g1: &G1Gadget,
        // These are just the generators for the Pedersen commitment and hash gadgets.
        pedersen_generators: &Vec<G1Gadget>,
    ) -> Result<(), SynthesisError> {
        // Verify that there are enough signers.
        self.check_signers(cs.ns(|| "check enough signers"), max_non_signers)?;

        // Get the hash point for the prepare round of signing.
        let prepare_hash = self.get_hash(
            cs.ns(|| "hash prepare round"),
            Round::Prepare,
            block_number,
            pks_commitment,
            sum_generator_g1,
            pedersen_generators,
        )?;

        // Get the hash point for the commit round of signing.
        let commit_hash = self.get_hash(
            cs.ns(|| "hash commit round"),
            Round::Commit,
            block_number,
            pks_commitment,
            sum_generator_g1,
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
            vec![prepare_agg_pk, commit_agg_pk],
            vec![&prepare_hash, &commit_hash],
            &signature,
            sig_generator,
        )
    }

    /// A function that calculates the hash for the block from:
    /// round number || block number || header_hash || pks_commitment
    /// where || means concatenation.
    /// First we use the Pedersen commitment to compress the input. Then we serialize the resulting
    /// EC point and hash it with the Blake2s hash algorithm, getting an output of 256 bits. This is
    /// necessary because the Pedersen commitment is not pseudo-random and we need pseudo-randomness
    /// for the BLS signature scheme. Finally we use the Pedersen hash algorithm on those 256 bits
    /// to obtain a single EC point.
    pub fn get_hash<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        &self,
        mut cs: CS,
        round: Round,
        block_number: &UInt32,
        pks_commitment: &Vec<UInt8>,
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

        // Convert the state commitment to bits and append it.
        let mut pks_bits = Vec::new();
        let mut byte;
        for i in 0..pks_commitment.len() {
            byte = pks_commitment[i].into_bits_le();
            byte.reverse();
            pks_bits.extend(byte);
        }
        bits.append(&mut pks_bits);

        // Calculate the Pedersen commitment.
        let pedersen_commitment = PedersenCommitmentGadget::evaluate(
            cs.ns(|| "pedersen commitment"),
            &bits,
            pedersen_generators,
            &sum_generator_g1,
        )?;

        // Serialize the Pedersen commitment.
        let x_bits = pedersen_commitment
            .x
            .to_bits(cs.ns(|| "x to bits: pedersen commitment"))?;
        let greatest_bit = YToBitGadget::y_to_bit_g1(
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
            &result,
            pedersen_generators,
            sum_generator_g1,
        )?;

        Ok(pedersen_result)
    }

    /// A function that checks if there are enough signers and if every signer in the commit round
    /// was also a signer in the prepare round.
    pub fn check_signers<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        &self,
        mut cs: CS,
        max_non_signers: &FqGadget,
    ) -> Result<(), SynthesisError> {
        // Initialize the running sum.
        let mut num_non_signers = FqGadget::zero(cs.ns(|| "number non signers"))?;

        // Conditionally add all other public keys.
        for i in 0..self.prepare_signer_bitmap.len() {
            // We only count a signer as valid if it signed both rounds.
            let valid = Boolean::and(
                cs.ns(|| format!("prepare bitmap & commit bitmap: {}", i)),
                &self.prepare_signer_bitmap[i],
                &self.commit_signer_bitmap[i],
            )?;

            // Update the number of non-signers. Note that the bitmap is negated to get the
            // non-signers: ~(included).
            num_non_signers = num_non_signers.conditionally_add_constant(
                cs.ns(|| format!("non-signers count {}", i)),
                &valid.not(),
                Fq::one(),
            )?;
        }

        // Enforce that there are enough signers. Specifically that:
        // num_non_signers <= max_non_signers
        num_non_signers.enforce_cmp(
            cs.ns(|| "enforce non signers"),
            max_non_signers,
            Ordering::Less,
            true,
        )
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

        assert_eq!(value.prepare_signer_bitmap.len(), VALIDATOR_SLOTS);
        assert_eq!(value.commit_signer_bitmap.len(), VALIDATOR_SLOTS);

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

        assert_eq!(value.prepare_signer_bitmap.len(), VALIDATOR_SLOTS);
        assert_eq!(value.commit_signer_bitmap.len(), VALIDATOR_SLOTS);

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
            prepare_signature,
            prepare_signer_bitmap,
            commit_signature,
            commit_signer_bitmap,
        })
    }
}
