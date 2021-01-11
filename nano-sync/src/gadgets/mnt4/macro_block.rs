use core::cmp::Ordering;
use std::borrow::Borrow;

use ark_crypto_primitives::prf::blake2s::constraints::{
    evaluate_blake2s_with_parameters, OutputVar,
};
use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ff::One;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var};
use ark_mnt6_753::Fq;
use ark_r1cs_std::alloc::AllocationMode;
use ark_r1cs_std::prelude::{
    AllocVar, Boolean, CondSelectGadget, FieldVar, ToBitsGadget, UInt32, UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, Namespace, SynthesisError};

use crate::constants::{MIN_SIGNERS, VALIDATOR_SLOTS};
use crate::gadgets::mnt4::{CheckSigGadget, PedersenHashGadget};
use crate::primitives::MacroBlock;
use crate::utils::reverse_inner_byte_order;

/// A gadget that contains utilities to verify the validity of a macro block. Mainly it checks that:
///  1. The macro block was signed by aggregate public keys.
///  2. The macro block contains the correct block number and public keys commitment (for the next
///     validator list).
///  3. There are enough signers.
pub struct MacroBlockGadget {
    pub header_hash: Vec<Boolean<MNT4Fr>>,
    pub signature: G1Var,
    pub signer_bitmap: Vec<Boolean<MNT4Fr>>,
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
        // The round number of the macro block.
        round_number: &UInt32<MNT4Fr>,
        // This is the aggregated public key.
        agg_pk: &G2Var,
        // These are just the generators for the Pedersen hash gadget.
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<(), SynthesisError> {
        // Verify that there are enough signers.
        self.check_signers(cs.clone())?;

        // Get the hash point for the signature.
        let hash = self.get_hash(
            block_number,
            round_number,
            pks_commitment,
            pedersen_generators,
        )?;

        // Verify the validity of the signature.
        CheckSigGadget::check_signature(cs, agg_pk, &hash, &self.signature)
    }

    /// A function that calculates the hash for the block from:
    /// step || block number || round number || header_hash || pks_commitment
    /// where || means concatenation.
    /// First we hash the input with the Blake2s hash algorithm, getting an output of 256 bits. This
    /// is necessary because the Pedersen commitment is not pseudo-random and we need pseudo-randomness
    /// for the BLS signature scheme. Then we use the Pedersen hash algorithm on those 256 bits
    /// to obtain a single EC point.
    pub fn get_hash(
        &self,
        block_number: &UInt32<MNT4Fr>,
        round_number: &UInt32<MNT4Fr>,
        pks_commitment: &Vec<UInt8<MNT4Fr>>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<G1Var, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits = vec![];

        // TODO: This first byte is the prefix for the precommit messages, it is the
        //       PREFIX_TENDERMINT_COMMIT constant in the nimiq_block_albatross crate. We can't
        //       import nimiq_block_albatross because of cyclic dependencies. When those constants
        //       get moved to the policy crate, we should import them here.
        let step = UInt8::constant(0x04);
        let mut step_bits = step.to_bits_be()?;
        bits.append(&mut step_bits);

        // The block number comes in little endian all the way. A reverse will put it into big endian.
        let mut block_number_bits = block_number.to_bits_le();

        block_number_bits.reverse();

        bits.append(&mut block_number_bits);

        // The round number comes in little endian all the way. A reverse will put it into big endian.
        let mut round_number_bits = round_number.to_bits_le();

        round_number_bits.reverse();

        bits.append(&mut round_number_bits);

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

        // Calculate first hash using Blake2s.
        let first_hash = evaluate_blake2s_with_parameters(&bits, &blake2s_parameters.parameters())?;

        // Convert to bits.
        let mut hash_bits = Vec::new();

        for i in 0..first_hash.len() {
            hash_bits.extend(first_hash[i].to_bits_le());
        }

        // Reverse inner byte order.
        let hash_bits = reverse_inner_byte_order(&hash_bits);

        // Feed the bits into the Pedersen hash to calculate the second hash.
        let second_hash = PedersenHashGadget::evaluate(&hash_bits, pedersen_generators)?;

        Ok(second_hash)
    }

    /// A function that checks if there are enough signers.
    pub fn check_signers(&self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Get the minimum number of signers.
        let min_signers = FqVar::new_constant(cs, &Fq::from(MIN_SIGNERS as u64))?;

        // Initialize the running sum.
        let mut num_signers = FqVar::zero();

        // Count the number of signers.
        for bit in &self.signer_bitmap {
            num_signers = CondSelectGadget::conditionally_select(
                bit,
                &(&num_signers + Fq::one()),
                &num_signers,
            )?;
        }

        // Enforce that there are enough signers. Specifically that:
        // num_signers >= min_signers
        num_signers.enforce_cmp(&min_signers, Ordering::Greater, true)
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

        assert_eq!(value.signer_bitmap.len(), VALIDATOR_SLOTS);

        let header_hash = OutputVar::new_input(cs.clone(), || Ok(&value.header_hash))?;

        // While the bytes of the Blake2sOutputGadget start with the most significant first,
        // the bits internally start with the least significant.
        // Thus, we need to reverse the bit order there.
        let header_hash = header_hash
            .0
            .into_iter()
            .flat_map(|n| reverse_inner_byte_order(&n.to_bits_le().unwrap()))
            .collect::<Vec<Boolean<MNT4Fr>>>();

        let signer_bitmap =
            Vec::<Boolean<MNT4Fr>>::new_input(cs.clone(), || Ok(&value.signer_bitmap[..]))?;

        let signature = G1Var::new_input(cs.clone(), || Ok(value.signature))?;

        Ok(MacroBlockGadget {
            header_hash,
            signature,
            signer_bitmap,
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

        assert_eq!(value.signer_bitmap.len(), VALIDATOR_SLOTS);

        let header_hash = OutputVar::new_witness(cs.clone(), || Ok(&value.header_hash))?;

        // While the bytes of the Blake2sOutputGadget start with the most significant first,
        // the bits internally start with the least significant.
        // Thus, we need to reverse the bit order there.
        let header_hash = header_hash
            .0
            .into_iter()
            .flat_map(|n| reverse_inner_byte_order(&n.to_bits_le().unwrap()))
            .collect::<Vec<Boolean<MNT4Fr>>>();

        let signer_bitmap =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(&value.signer_bitmap[..]))?;

        let signature = G1Var::new_witness(cs.clone(), || Ok(value.signature))?;

        Ok(MacroBlockGadget {
            header_hash,
            signature,
            signer_bitmap,
        })
    }
}
