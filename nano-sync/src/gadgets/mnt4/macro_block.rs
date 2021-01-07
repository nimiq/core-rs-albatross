use core::cmp::Ordering;
use std::borrow::Borrow;

use ark_crypto_primitives::prf::blake2s::constraints::{
    evaluate_blake2s_with_parameters, OutputVar,
};
use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ff::One;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var};
use ark_mnt6_753::{Fq, FqParameters};
use ark_r1cs_std::prelude::{
    AllocVar, Boolean, CondSelectGadget, FieldVar, ToBitsGadget, UInt32, UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, Namespace, SynthesisError};

use crate::constants::VALIDATOR_SLOTS;
use crate::gadgets::mnt4::{CheckSigGadget, PedersenHashGadget, YToBitGadget};
use crate::primitives::MacroBlock;
use crate::utils::{pad_point_bits, reverse_inner_byte_order};
use ark_r1cs_std::alloc::AllocationMode;

/// A simple enum representing the two rounds of signing in the macro blocks.
#[derive(Clone, Copy, Ord, PartialOrd, PartialEq, Eq)]
pub enum Round {
    Prepare = 0,
    Commit = 1,
}

/// A gadget that contains utilities to verify the validity of a macro block. Mainly it checks that:
///  1. The macro block was signed by the prepare and commit aggregate public keys.
///  2. The macro block contains the correct block number and public keys commitment (for the next
///     validator list).
///  3. There are enough signers.
pub struct MacroBlockGadget {
    pub header_hash: Vec<Boolean<MNT4Fr>>,
    pub prepare_signature: G1Var,
    pub prepare_signer_bitmap: Vec<Boolean<MNT4Fr>>,
    pub commit_signature: G1Var,
    pub commit_signer_bitmap: Vec<Boolean<MNT4Fr>>,
}

impl MacroBlockGadget {
    /// A function that verifies the validity of a given macro block. It is the main function for
    /// the macro block gadget.
    pub fn verify(
        &self,
        cs: ConstraintSystemRef<MNT4Fr>,
        // This is the commitment for the set of public keys that are owned by the next set of validators.
        pks_commitment: &Vec<UInt8<MNT4Fr>>,
        // Simply the number of the macro block.
        block_number: &UInt32<MNT4Fr>,
        // This is the aggregated public key for the prepare round.
        prepare_agg_pk: &G2Var,
        // This is the aggregated public key for the commit round.
        commit_agg_pk: &G2Var,
        // This is the maximum number of non-signers for the block. It is inclusive, meaning that if
        // the number of non-signers == max_non-signers then the block is still valid.
        max_non_signers: &FqVar,
        // The generator used in the BLS signature scheme. It is the generator used to create public
        // keys.
        sig_generator: &G2Var,
        // The next generator is only needed because the elliptic curve addition in-circuit is
        // incomplete. Meaning that it can't handle the identity element (aka zero, aka point-at-infinity).
        // So, this generator is needed to do running sums. Instead of starting at zero, we start
        // with the generator and subtract it at the end of the running sum.
        sum_generator_g1: &G1Var,
        // These are just the generators for the Pedersen hash gadget.
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<(), SynthesisError> {
        // Verify that there are enough signers.
        self.check_signers(max_non_signers)?;

        // Get the hash point for the prepare round of signing.
        let prepare_hash = self.get_hash(
            cs.clone(),
            Round::Prepare,
            block_number,
            pks_commitment,
            pedersen_generators,
        )?;

        // Get the hash point for the commit round of signing.
        let commit_hash = self.get_hash(
            cs.clone(),
            Round::Commit,
            block_number,
            pks_commitment,
            pedersen_generators,
        )?;

        // Add together the two aggregated signatures for the prepare and commit rounds of signing.
        // Note the use of the generator to avoid an error in the sum.
        let mut signature = sum_generator_g1 + self.prepare_signature.clone();

        signature = signature + self.commit_signature.clone();

        signature = signature - sum_generator_g1;

        // Verifies the validity of the signatures.
        CheckSigGadget::check_signatures(
            &[prepare_agg_pk, commit_agg_pk],
            &[&prepare_hash, &commit_hash],
            &signature,
            sig_generator,
        )
    }

    /// A function that calculates the hash for the block from:
    /// round number || block number || header_hash || pks_commitment
    /// where || means concatenation.
    /// First we use the Pedersen hash function to compress the input. Then we serialize the resulting
    /// EC point and hash it with the Blake2s hash algorithm, getting an output of 256 bits. This is
    /// necessary because the Pedersen hash is not pseudo-random and we need pseudo-randomness
    /// for the BLS signature scheme. Finally we use the Pedersen hash again on those 256 bits
    /// to obtain a single EC point.
    pub fn get_hash(
        &self,
        cs: ConstraintSystemRef<MNT4Fr>,
        round: Round,
        block_number: &UInt32<MNT4Fr>,
        pks_commitment: &Vec<UInt8<MNT4Fr>>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<G1Var, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits = vec![];

        // The round number comes in little endian,
        // which is why we need to reverse the bits to get big endian.
        let round_number = UInt8::constant(round as u8);

        let mut round_number_bits = round_number.to_bits_le()?;

        round_number_bits.reverse();

        bits.append(&mut round_number_bits);

        // The block number comes in little endian all the way.
        // So, a reverse will put it into big endian.
        let mut block_number_be = block_number.to_bits_le();

        block_number_be.reverse();

        bits.append(&mut block_number_be);

        // Append the header hash.
        bits.extend_from_slice(&self.header_hash);

        // Convert the public keys commitment to bits and append it.
        let mut pks_bits = Vec::new();

        let mut byte;

        for i in 0..pks_commitment.len() {
            byte = pks_commitment[i].to_bits_le()?;
            byte.reverse();
            pks_bits.extend(byte);
        }

        bits.append(&mut pks_bits);

        // Calculate the first hash using Pedersen.
        let first_hash = PedersenHashGadget::evaluate(&bits, pedersen_generators)?;

        // Serialize the Pedersen hash.
        let x_bits = first_hash.x.to_bits_le()?;

        let greatest_bit = YToBitGadget::y_to_bit_g1(cs.clone(), &first_hash)?;

        let serialized_bits = pad_point_bits::<FqParameters, MNT4Fr>(x_bits, greatest_bit);

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

        // Calculate second hash using Blake2s.
        let second_hash =
            evaluate_blake2s_with_parameters(&serialized_bits, &blake2s_parameters.parameters())?;

        // Convert to bits.
        let mut hash_bits = Vec::new();

        for i in 0..second_hash.len() {
            hash_bits.extend(second_hash[i].to_bits_le());
        }

        // Reverse inner byte order (again).
        let hash_bits = reverse_inner_byte_order(&hash_bits);

        // Finally feed the bits into the Pedersen hash to calculate the third hash.
        let third_hash = PedersenHashGadget::evaluate(&hash_bits, pedersen_generators)?;

        Ok(third_hash)
    }

    /// A function that checks if there are enough signers and if every signer in the commit round
    /// was also a signer in the prepare round.
    pub fn check_signers(&self, max_non_signers: &FqVar) -> Result<(), SynthesisError> {
        // Initialize the running sum.
        let mut num_non_signers = FqVar::zero();

        // Conditionally add all other public keys.
        for i in 0..self.prepare_signer_bitmap.len() {
            // We only count a signer as valid if it signed both rounds.
            let valid = Boolean::and(
                &self.prepare_signer_bitmap[i],
                &self.commit_signer_bitmap[i],
            )?;

            // Update the number of non-signers. Note that the bitmap is negated to get the
            // non-signers: ~(included).
            num_non_signers = CondSelectGadget::conditionally_select(
                &valid.not(),
                &(num_non_signers.clone() + Fq::one()),
                &num_non_signers,
            )?;
        }

        // Enforce that there are enough signers. Specifically that:
        // num_non_signers <= max_non_signers
        num_non_signers.enforce_cmp(max_non_signers, Ordering::Less, true)
    }
}

/// The allocation function for the macro block gadget.
impl AllocVar<MacroBlock, MNT4Fr> for MacroBlockGadget {
    fn new_variable<T: Borrow<MacroBlock>>(
        cs: impl Into<Namespace<MNT4Fr>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: AllocationMode,
    ) -> Result<Self, SynthesisError> {
        match mode {
            AllocationMode::Constant => unreachable!(),
            AllocationMode::Input => Self::new_input(cs, f),
            AllocationMode::Witness => Self::new_witness(cs, f),
        }
    }

    fn new_input<T: Borrow<MacroBlock>>(
        cs: impl Into<Namespace<MNT4Fr>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();

        let empty_block = MacroBlock::default();

        let value = match f() {
            Ok(val) => val.borrow().clone(),
            Err(_) => empty_block,
        };

        assert_eq!(value.prepare_signer_bitmap.len(), VALIDATOR_SLOTS);

        assert_eq!(value.commit_signer_bitmap.len(), VALIDATOR_SLOTS);

        let header_hash = OutputVar::new_input(cs.clone(), || Ok(&value.header_hash))?;

        // While the bytes of the Blake2sOutputGadget start with the most significant first,
        // the bits internally start with the least significant.
        // Thus, we need to reverse the bit order there.
        let header_hash = header_hash
            .0
            .into_iter()
            .flat_map(|n| reverse_inner_byte_order(&n.to_bits_le().unwrap()))
            .collect::<Vec<Boolean<MNT4Fr>>>();

        let prepare_signer_bitmap =
            Vec::<Boolean<MNT4Fr>>::new_input(cs.clone(), || Ok(&value.prepare_signer_bitmap[..]))?;

        let prepare_signature = G1Var::new_input(cs.clone(), || Ok(value.prepare_signature))?;

        let commit_signer_bitmap =
            Vec::<Boolean<MNT4Fr>>::new_input(cs.clone(), || Ok(&value.commit_signer_bitmap[..]))?;

        let commit_signature = G1Var::new_input(cs.clone(), || Ok(value.commit_signature))?;

        Ok(MacroBlockGadget {
            header_hash,
            prepare_signature,
            prepare_signer_bitmap,
            commit_signature,
            commit_signer_bitmap,
        })
    }

    fn new_witness<T: Borrow<MacroBlock>>(
        cs: impl Into<Namespace<MNT4Fr>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();

        let empty_block = MacroBlock::default();

        let value = match f() {
            Ok(val) => val.borrow().clone(),
            Err(_) => empty_block,
        };

        assert_eq!(value.prepare_signer_bitmap.len(), VALIDATOR_SLOTS);

        assert_eq!(value.commit_signer_bitmap.len(), VALIDATOR_SLOTS);

        let header_hash = OutputVar::new_witness(cs.clone(), || Ok(&value.header_hash))?;

        // While the bytes of the Blake2sOutputGadget start with the most significant first,
        // the bits internally start with the least significant.
        // Thus, we need to reverse the bit order there.
        let header_hash = header_hash
            .0
            .into_iter()
            .flat_map(|n| reverse_inner_byte_order(&n.to_bits_le().unwrap()))
            .collect::<Vec<Boolean<MNT4Fr>>>();

        let prepare_signer_bitmap = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || {
            Ok(&value.prepare_signer_bitmap[..])
        })?;

        let prepare_signature = G1Var::new_witness(cs.clone(), || Ok(value.prepare_signature))?;

        let commit_signer_bitmap = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || {
            Ok(&value.commit_signer_bitmap[..])
        })?;

        let commit_signature = G1Var::new_witness(cs.clone(), || Ok(value.commit_signature))?;

        Ok(MacroBlockGadget {
            header_hash,
            prepare_signature,
            prepare_signer_bitmap,
            commit_signature,
            commit_signer_bitmap,
        })
    }
}
