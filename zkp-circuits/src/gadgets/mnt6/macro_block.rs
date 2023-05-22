use core::cmp::Ordering;
use std::borrow::Borrow;

use ark_crypto_primitives::prf::blake2s::constraints::OutputVar;
use ark_ff::One;
use ark_mnt6_753::{
    constraints::{FqVar, G1Var, G2Var},
    Fq as MNT6Fq,
};
use ark_r1cs_std::{
    alloc::AllocationMode,
    prelude::{AllocVar, Boolean, CondSelectGadget, FieldVar, UInt32, UInt8},
};
use ark_relations::r1cs::{ConstraintSystemRef, Namespace, SynthesisError};
use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::MacroBlock;

use crate::{
    blake2s::evaluate_blake2s,
    gadgets::{
        be_bytes::ToBeBytesGadget,
        bits::BitVec,
        mnt6::{CheckSigGadget, HashToCurve},
    },
};

/// A gadget that contains utilities to verify the validity of a macro block. Mainly it checks that:
///  1. The macro block was signed by the aggregate public key.
///  2. The macro block contains the correct block number and public keys commitment (for the next
///     validator list).
///  3. There are enough signers.
pub struct MacroBlockGadget {
    pub block_number: UInt32<MNT6Fq>,
    pub round_number: UInt32<MNT6Fq>,
    pub header_hash: Vec<UInt8<MNT6Fq>>,
    pub signature: G1Var,
    pub signer_bitmap: Vec<Boolean<MNT6Fq>>,
}

impl MacroBlockGadget {
    /// A function that verifies the validity of a given macro block. It is the main function for
    /// the macro block gadget.
    pub fn verify(
        &self,
        cs: ConstraintSystemRef<MNT6Fq>,
        // This is the commitment for the set of public keys that are owned by the next set of validators.
        pk_tree_root: &[UInt8<MNT6Fq>],
        // This is the aggregated public key.
        agg_pk: &G2Var,
    ) -> Result<Boolean<MNT6Fq>, SynthesisError> {
        // Verify that there are enough signers.
        let enough_signers = self.check_signers(cs.clone())?;

        // Get the hash point for the signature.
        let hash = self.get_hash(cs.clone())?;

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
    pub fn get_hash(&self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<G1Var, SynthesisError> {
        // Initialize Boolean vector.
        let mut second_bytes = vec![];

        // Add the tendermint step.
        second_bytes.push(UInt8::constant(MacroBlock::TENDERMINT_STEP));

        // The round number.
        let mut round_number_bytes = self.round_number.to_bytes_be()?;
        second_bytes.append(&mut round_number_bytes);

        // The block number.
        let mut block_number_bits = self.block_number.to_bytes_be()?;
        second_bytes.append(&mut block_number_bits);

        // Add header hash represented as an option.
        second_bytes.push(UInt8::constant(0x01));
        second_bytes.extend_from_slice(&self.header_hash);

        // Calculate hash using Blake2s.
        let second_hash = evaluate_blake2s(&second_bytes)?;

        // Hash-to-curve.
        let g1_point = HashToCurve::hash_to_g1(cs, &second_hash)?;

        Ok(g1_point)
    }

    /// A function that checks if there are enough signers.
    pub fn check_signers(
        &self,
        cs: ConstraintSystemRef<MNT6Fq>,
    ) -> Result<Boolean<MNT6Fq>, SynthesisError> {
        // Get the minimum number of signers.
        let min_signers = FqVar::new_constant(cs, MNT6Fq::from(Policy::TWO_F_PLUS_ONE as u64))?;

        // Initialize the running sum.
        let mut num_signers = FqVar::zero();

        // Count the number of signers.
        for bit in &self.signer_bitmap {
            num_signers = CondSelectGadget::conditionally_select(
                bit,
                &(&num_signers + MNT6Fq::one()),
                &num_signers,
            )?;
        }

        // Enforce that there are enough signers. Specifically that:
        // num_signers >= min_signers
        num_signers.is_cmp(&min_signers, Ordering::Greater, true)
    }
}

/// The allocation function for the macro block gadget.
impl AllocVar<MacroBlock, MNT6Fq> for MacroBlockGadget {
    fn new_variable<T: Borrow<MacroBlock>>(
        cs: impl Into<Namespace<MNT6Fq>>,
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
        cs: impl Into<Namespace<MNT6Fq>>,
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

        let block_number = UInt32::<MNT6Fq>::new_input(cs.clone(), || Ok(value.block_number))?;

        let round_number = UInt32::<MNT6Fq>::new_input(cs.clone(), || Ok(value.round_number))?;

        let header_hash = OutputVar::new_input(cs.clone(), || Ok(&value.header_hash.0))?;
        let header_hash = header_hash.0;

        let signer_bitmap = BitVec::<MNT6Fq>::new_input_vec(cs.clone(), &value.signer_bitmap[..])?;
        let signer_bitmap = signer_bitmap.0;

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
        cs: impl Into<Namespace<MNT6Fq>>,
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

        let block_number = UInt32::<MNT6Fq>::new_witness(cs.clone(), || Ok(value.block_number))?;

        let round_number = UInt32::<MNT6Fq>::new_witness(cs.clone(), || Ok(value.round_number))?;

        let header_hash = OutputVar::new_witness(cs.clone(), || Ok(&value.header_hash.0))?;
        let header_hash = header_hash.0;

        let signer_bitmap =
            BitVec::<MNT6Fq>::new_witness_vec(cs.clone(), &value.signer_bitmap[..])?;
        let signer_bitmap = signer_bitmap.0;

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
    use ark_ec::Group;
    use ark_ff::Zero;
    use ark_mnt6_753::{constraints::G2Var, Fq as MNT6Fq, Fr, G1Projective, G2Projective};
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{ops::MulAssign, test_rng, UniformRand};
    use nimiq_hash::Blake2sHash;
    use nimiq_primitives::policy::Policy;
    use nimiq_test_log::test;
    use rand::{Rng, RngCore};

    use super::*;

    #[test]
    fn block_hash_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create block parameters.
        let mut bytes = [1u8; 95];
        rng.fill_bytes(&mut bytes);

        let mut header_hash = [2u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut signer_bitmap = Vec::with_capacity(Policy::SLOTS as usize);
        for _ in 0..Policy::SLOTS {
            signer_bitmap.push(rng.gen());
        }

        let block = MacroBlock {
            block_number: u32::rand(rng),
            round_number: u32::rand(rng),
            header_hash,
            signature: G1Projective::rand(rng),
            signer_bitmap,
        };

        // Calculate hash using the primitive version.
        let primitive_hash = block.hash();

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        // Calculate hash using the gadget version.
        let gadget_hash = block_var.get_hash(cs).unwrap();

        assert_eq!(primitive_hash, gadget_hash.value().unwrap())
    }

    #[test]
    fn block_verify() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Create wrong block number.
        block.block_number += 1;

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Create wrong round number.
        block.round_number += 1;

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Create wrong public keys tree root.
        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Create wrong agg pk.
        let agg_pk = G2Projective::rand(rng);

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Create wrong header hash.
        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);
        block.header_hash = header_hash;

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with correct signers set.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Create wrong signature.
        block.signature = G1Projective::rand(rng);

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let sk = Fr::rand(rng);
        let mut pk = G2Projective::generator();
        pk.mul_assign(sk);

        // Create more block parameters.
        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let pk_tree_root = bytes.to_vec();

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);
        let header_hash = Blake2sHash::from(header_hash);

        let mut agg_pk = G2Projective::zero();

        // Create macro block with too few signers.
        let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

        for i in 0..Policy::TWO_F_PLUS_ONE as usize - 1 {
            block.sign(&sk, i);
            agg_pk += &pk;
        }

        // Allocate parameters in the circuit.
        let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify(cs, &pk_tree_root_var, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }
}
