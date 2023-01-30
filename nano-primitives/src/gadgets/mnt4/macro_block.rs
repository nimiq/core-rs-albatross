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

use nimiq_primitives::policy::Policy;

use crate::gadgets::mnt4::{CheckSigGadget, HashToCurve};
use crate::utils::reverse_inner_byte_order;
use crate::MacroBlock;

/// A gadget that contains utilities to verify the validity of a macro block. Mainly it checks that:
///  1. The macro block was signed by the aggregate public key.
///  2. The macro block contains the correct block number and public keys commitment (for the next
///     validator list).
///  3. There are enough signers.
pub struct MacroBlockGadget {
    pub block_number: UInt32<MNT4Fr>,
    pub round_number: UInt32<MNT4Fr>,
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
        pk_tree_root: &[Boolean<MNT4Fr>],
        // This is the aggregated public key.
        agg_pk: &G2Var,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        // Verify that there are enough signers.
        let enough_signers = self.check_signers(cs.clone())?;

        // Get the hash point for the signature.
        let hash = self.get_hash(cs.clone(), pk_tree_root)?;

        // Check the validity of the signature.
        let valid_sig = CheckSigGadget::check_signature(cs, agg_pk, &hash, &self.signature)?;

        // Only return true if we have enough signers and a valid signature.
        enough_signers.and(&valid_sig)
    }

    /// A function that calculates the hash point for the block. This should match exactly the hash
    /// point used in validator's signatures. It works like this:
    ///     1. Get the header hash and the pk_tree_root.
    ///     2. Calculate the first hash like so:
    ///             first_hash = Blake2s( header_hash || pk_tree_root )
    ///     3. Calculate the second (and final) hash like so:
    ///             second_hash = Blake2s( 0x04 || round number || block number || 0x01 || first_hash )
    ///        The first four fields (0x04, round number, block number, 0x01) are needed for the
    ///        Tendermint protocol and there is no reason to explain their meaning here.
    ///     4. Finally, we take the second hash and map it to an elliptic curve point using the
    ///        "try-and-increment" method.
    /// The function || means concatenation.
    pub fn get_hash(
        &self,
        cs: ConstraintSystemRef<MNT4Fr>,
        pk_tree_root: &[Boolean<MNT4Fr>],
    ) -> Result<G1Var, SynthesisError> {
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

        // Initialize Boolean vector.
        let mut first_bits = vec![];

        // Append the header hash.
        first_bits.extend_from_slice(&self.header_hash);

        // Append the public key tree root.
        first_bits.extend_from_slice(pk_tree_root);

        // Prepare order of booleans for blake2s (it doesn't expect Big-Endian)!
        let prepared_first_bits = reverse_inner_byte_order(&first_bits);

        // Calculate hash using Blake2s.
        let first_hash = evaluate_blake2s_with_parameters(
            &prepared_first_bits,
            &blake2s_parameters.parameters(),
        )?;

        // Convert to bits.
        let mut first_hash_bits = Vec::new();

        for int in &first_hash {
            first_hash_bits.extend(int.to_bits_le());
        }

        // Reverse inner-byte order again.
        let mut first_hash_bits = reverse_inner_byte_order(&first_hash_bits);

        // Initialize Boolean vector.
        let mut second_bits = vec![];

        // Add the first byte.
        let byte = UInt8::constant(0x04);

        let mut bits = byte.to_bits_be()?;

        second_bits.append(&mut bits);

        // The round number comes in little endian all the way. A reverse will put it into big endian.
        let mut round_number_bits = self.round_number.clone().to_bits_le();

        round_number_bits.reverse();

        second_bits.append(&mut round_number_bits);

        // The block number comes in little endian all the way. A reverse will put it into big endian.
        let mut block_number_bits = self.block_number.clone().to_bits_le();

        block_number_bits.reverse();

        second_bits.append(&mut block_number_bits);

        // Add another byte.
        let byte = UInt8::constant(0x01);

        let mut bits = byte.to_bits_be()?;

        second_bits.append(&mut bits);

        // Append the first hash.
        second_bits.append(&mut first_hash_bits);

        // Prepare order of booleans for blake2s (it doesn't expect Big-Endian)!
        let prepared_second_bits = reverse_inner_byte_order(&second_bits);

        // Calculate hash using Blake2s.
        let second_hash = evaluate_blake2s_with_parameters(
            &prepared_second_bits,
            &blake2s_parameters.parameters(),
        )?;

        // Convert to bits.
        let mut second_hash_bits = Vec::new();

        for int in &second_hash {
            second_hash_bits.extend(int.to_bits_le());
        }

        // At this point the hash does not match the off-circuit one. It has the inner byte order
        // reversed. However we need it like this for the next step.

        // Hash-to-curve.
        let g1_point = HashToCurve::hash_to_g1(cs, &second_hash_bits)?;

        Ok(g1_point)
    }

    /// A function that checks if there are enough signers.
    pub fn check_signers(
        &self,
        cs: ConstraintSystemRef<MNT4Fr>,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        // Get the minimum number of signers.
        let min_signers = FqVar::new_constant(cs, Fq::from(Policy::TWO_F_PLUS_ONE as u64))?;

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
        num_signers.is_cmp(&min_signers, Ordering::Greater, true)
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

        assert_eq!(value.signer_bitmap.len(), Policy::SLOTS as usize);

        let block_number = UInt32::<MNT4Fr>::new_input(cs.clone(), || Ok(value.block_number))?;

        let round_number = UInt32::<MNT4Fr>::new_input(cs.clone(), || Ok(value.round_number))?;

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

        let signature = G1Var::new_input(cs, || Ok(value.signature))?;

        Ok(MacroBlockGadget {
            block_number,
            round_number,
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

        assert_eq!(value.signer_bitmap.len(), Policy::SLOTS as usize);

        let block_number = UInt32::<MNT4Fr>::new_witness(cs.clone(), || Ok(value.block_number))?;

        let round_number = UInt32::<MNT4Fr>::new_witness(cs.clone(), || Ok(value.round_number))?;

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

        let signature = G1Var::new_witness(cs, || Ok(value.signature))?;

        Ok(MacroBlockGadget {
            block_number,
            round_number,
            header_hash,
            signature,
            signer_bitmap,
        })
    }
}

#[cfg(test)]
mod tests {
    use ark_ec::ProjectiveCurve;
    use ark_ff::Zero;
    use ark_mnt4_753::Fr as MNT4Fr;
    use ark_mnt6_753::constraints::G2Var;
    use ark_mnt6_753::{Fr, G1Projective, G2Projective};
    use ark_r1cs_std::prelude::{AllocVar, Boolean};
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::ops::MulAssign;
    use ark_std::{test_rng, UniformRand};
    use rand::RngCore;

    use nimiq_bls::utils::bytes_to_bits;
    use nimiq_primitives::policy::Policy;
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn block_hash_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create block parameters.
        let mut bytes = [1u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [2u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut bytes = [3u8; Policy::SLOTS as usize / 8];
        rng.fill_bytes(&mut bytes);
        let signer_bitmap = bytes_to_bits(&bytes);

        let block = MacroBlock {
            block_number: u32::rand(rng),
            round_number: u32::rand(rng),
            header_hash,
            signature: G1Projective::rand(rng),
            signer_bitmap,
        };

        // Calculate hash using the primitive version.
        let primitive_hash = block.hash(&pk_tree_root);

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        // Calculate hash using the gadget version.
        let gadget_hash = block_var.get_hash(cs, &pk_tree_root_var).unwrap();

        assert_eq!(primitive_hash, gadget_hash.value().unwrap())
    }

    #[test]
    fn block_verify() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_block_number() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Create wrong block number.
        block.block_number += 1;

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_round_number() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Create wrong round number.
        block.round_number += 1;

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_pk_tree_root() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Create wrong public keys tree root.
        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_agg_pk() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Create wrong agg pk.
        let agg_pk = G2Projective::rand(rng);

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_header_hash() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Create wrong header hash.
        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        block.header_hash = header_hash;

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_signature() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Create wrong signature.
        block.signature = G1Projective::rand(rng);

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_too_few_signers() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::prime_subgroup_generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with too few signers.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize - 1 {
            block.sign(&sk, i, &pk_tree_root);
            agg_pk += &pk;
        }

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
                .unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }
}
