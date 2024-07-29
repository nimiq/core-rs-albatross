use core::cmp::Ordering;
use std::borrow::Borrow;

use ark_ff::One;
use ark_mnt6_753::{
    constraints::{FqVar, G1Var, G2Var},
    Fq as MNT6Fq, G1Projective,
};
use ark_r1cs_std::{
    alloc::AllocationMode,
    prelude::{AllocVar, Boolean, CondSelectGadget, FieldVar, UInt32, UInt8},
    uint16::UInt16,
};
use ark_relations::r1cs::{ConstraintSystemRef, Namespace, SynthesisError};
use ark_std::Zero;
use nimiq_block::MacroBlock;
use nimiq_hash::{Blake2sHash, Hash, HashOutput, SerializeContent};
use nimiq_primitives::{policy::Policy, TendermintStep};

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
    pub network: UInt8<MNT6Fq>,
    pub version: UInt16<MNT6Fq>,
    pub block_number: UInt32<MNT6Fq>,
    /// Encompasses everything in between timestamp and body root
    pub intermediate_header_bytes: Vec<UInt8<MNT6Fq>>,
    pub history_root: Vec<UInt8<MNT6Fq>>,

    // Body
    pub pk_tree_root: Vec<UInt8<MNT6Fq>>,
    /// Encompasses everything after the validators object
    pub body_bytes: Vec<UInt8<MNT6Fq>>,

    // Signature
    pub signer_bitmap: Vec<Boolean<MNT6Fq>>,
    pub signature: G1Var,
    /// The round number for the justification.
    pub round: UInt32<MNT6Fq>,

    /// Caching a computed header hash.
    /// We always recompute it the first time, to make sure it's correct.
    header_hash: Option<Vec<UInt8<MNT6Fq>>>,
}

impl MacroBlockGadget {
    /// A function that verifies the validity of a given macro block. It is the main function for
    /// the macro block gadget.
    pub fn verify_signature(
        &mut self,
        cs: ConstraintSystemRef<MNT6Fq>,
        // This is the aggregated public key.
        agg_pk: &G2Var,
    ) -> Result<Boolean<MNT6Fq>, SynthesisError> {
        // Verify that there are enough signers.
        let enough_signers = self.check_signers(cs.clone())?;

        // Get the hash point for the signature.
        let hash = self.tendermint_hash(cs.clone())?;

        // Check the validity of the signature.
        let valid_sig = CheckSigGadget::check_signature(cs, agg_pk, &hash, &self.signature)?;

        // Only return true if we have enough signers and a valid signature.
        enough_signers.and(&valid_sig)
    }

    /// Calculates the header hash of the block.
    //
    // Needs to be kept in sync with `MacroHeader::serialize_content`.
    pub fn hash(
        &mut self,
        _cs: ConstraintSystemRef<MNT6Fq>,
    ) -> Result<&[UInt8<MNT6Fq>], SynthesisError> {
        if let Some(ref header_hash) = self.header_hash {
            return Ok(header_hash);
        }

        // Initialize bytes to be hashed for the body hash.
        let mut body_bytes = vec![];
        body_bytes.extend_from_slice(&self.pk_tree_root);
        body_bytes.extend_from_slice(&self.body_bytes);
        let mut body_hash = evaluate_blake2s(&body_bytes)?;

        // Initialize bytes to be hashed for the header hash.
        let mut header_bytes = vec![];
        header_bytes.push(self.network.clone());
        header_bytes.append(&mut self.version.to_bytes_be()?);
        header_bytes.append(&mut self.block_number.to_bytes_be()?);
        header_bytes.extend_from_slice(&self.intermediate_header_bytes);
        header_bytes.append(&mut body_hash);
        header_bytes.extend_from_slice(&self.history_root);
        let header_hash = evaluate_blake2s(&header_bytes)?;

        // Cache calculated hash.
        Ok(self.header_hash.insert(header_hash))
    }

    /// A function that calculates the hash point for the block. This should match exactly the hash
    /// point used in validator's signatures. It works like this:
    ///     1. Get the header hash and the pk_tree_root.
    ///     2. Calculate the first hash like so:
    ///             first_hash = Blake2s( header_hash || pk_tree_root )
    ///     3. Calculate the second (and final) hash like so:
    ///             second_hash = Blake2s( network_id || 0x04 || round number || block number || 0x01 || first_hash )
    ///        The first four fields (network_id, 0x04, round number, block number, 0x01) are needed
    ///        for the Tendermint protocol and there is no reason to explain their meaning here.
    ///     4. Finally, we take the second hash and map it to an elliptic curve point using the
    ///        "try-and-increment" method.
    /// The function || means concatenation.
    //
    // Needs to be kept in sync with `TendermintVote::serialize_content`.
    pub fn tendermint_hash(
        &mut self,
        cs: ConstraintSystemRef<MNT6Fq>,
    ) -> Result<G1Var, SynthesisError> {
        // Initialize Boolean vector.
        let mut bytes = vec![];

        // Add the tendermint step.
        bytes.push(UInt8::constant(TendermintStep::PreCommit as u8));

        // The network ID.
        bytes.push(self.network.clone());

        // The round number.
        let mut round_number_bytes = self.round.to_bytes_be()?;
        bytes.append(&mut round_number_bytes);

        // The block number.
        let mut block_number_bits = self.block_number.to_bytes_be()?;
        bytes.append(&mut block_number_bits);

        // Add header hash represented as an option.
        bytes.push(UInt8::constant(0x01));
        bytes.extend_from_slice(self.hash(cs.clone())?);

        // Calculate hash using Blake2s.
        let hash = evaluate_blake2s(&bytes)?;

        // Hash-to-curve.
        let g1_point = HashToCurve::hash_to_g1(cs, &hash)?;

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

        let value = match f() {
            Ok(val) => val.borrow().clone(),
            Err(_) => MacroBlock::non_empty_default(),
        };

        let signature;
        let signer_bitmap: Vec<bool>;
        let justification_round;
        if let Some(ref justification) = value.justification {
            signature = justification.sig.signature.get_point();
            signer_bitmap = justification
                .sig
                .signers
                .iter_bits()
                .take(Policy::SLOTS as usize)
                .collect();
            justification_round = justification.round;
        } else {
            signature = G1Projective::zero();
            signer_bitmap = vec![false; Policy::SLOTS as usize];
            justification_round = 0;
        }

        let network = UInt8::<MNT6Fq>::new_input(cs.clone(), || Ok(value.header.network as u8))?;
        let version = UInt16::<MNT6Fq>::new_input(cs.clone(), || Ok(value.header.version))?;
        let block_number =
            UInt32::<MNT6Fq>::new_input(cs.clone(), || Ok(value.header.block_number))?;
        let mut header_bytes = Vec::new();
        value
            .header
            .serialize_content::<_, Blake2sHash>(&mut header_bytes)
            .map_err(|_| SynthesisError::AssignmentMissing)?;

        // Get all bytes starting after the round number until the last two hashes.
        let intermediate_header_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &header_bytes[7..header_bytes.len() - 64])?;
        let history_root =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), value.header.history_root.as_bytes())?;

        // Body
        let validators = value
            .header
            .validators
            .as_ref()
            .ok_or(SynthesisError::AssignmentMissing)?;

        let mut body_bytes = Vec::new();
        value
            .header
            .zkp_body_serialize_content(&mut body_bytes)
            .map_err(|_| SynthesisError::AssignmentMissing)?;

        let pk_tree_root =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &validators.hash::<Blake2sHash>().0)?;
        let body_bytes = UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &body_bytes[32..])?;

        // Signature
        let signer_bitmap = BitVec::<MNT6Fq>::new_input_vec(cs.clone(), &signer_bitmap[..])?;
        let signer_bitmap = signer_bitmap.0;

        let signature = G1Var::new_input(cs.clone(), || Ok(signature))?;
        let round = UInt32::<MNT6Fq>::new_input(cs, || Ok(justification_round))?;

        Ok(MacroBlockGadget {
            network,
            version,
            block_number,
            intermediate_header_bytes,
            history_root,
            pk_tree_root,
            body_bytes,
            signature,
            signer_bitmap,
            round,
            header_hash: None,
        })
    }

    fn new_witness<T: Borrow<MacroBlock>>(
        cs: impl Into<Namespace<MNT6Fq>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();

        let value = match f() {
            Ok(val) => val.borrow().clone(),
            Err(_) => MacroBlock::non_empty_default(),
        };

        let signature;
        let signer_bitmap: Vec<bool>;
        let justification_round;
        if let Some(ref justification) = value.justification {
            signature = justification.sig.signature.get_point();
            signer_bitmap = justification
                .sig
                .signers
                .iter_bits()
                .take(Policy::SLOTS as usize)
                .collect();
            justification_round = justification.round;
        } else {
            signature = G1Projective::zero();
            signer_bitmap = vec![false; Policy::SLOTS as usize];
            justification_round = 0;
        }

        let network = UInt8::<MNT6Fq>::new_witness(cs.clone(), || Ok(value.header.network as u8))?;
        let version = UInt16::<MNT6Fq>::new_witness(cs.clone(), || Ok(value.header.version))?;
        let block_number =
            UInt32::<MNT6Fq>::new_witness(cs.clone(), || Ok(value.header.block_number))?;

        let mut header_bytes = Vec::new();
        value
            .header
            .serialize_content::<_, Blake2sHash>(&mut header_bytes)
            .map_err(|_| SynthesisError::AssignmentMissing)?;

        // Get all bytes starting after the round number until the last two hashes.
        let intermediate_header_bytes = UInt8::<MNT6Fq>::new_witness_vec(
            cs.clone(),
            &header_bytes[7..header_bytes.len() - 64],
        )?;
        let history_root =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), value.header.history_root.as_bytes())?;

        // Body
        let validators = value
            .header
            .validators
            .as_ref()
            .ok_or(SynthesisError::AssignmentMissing)?;

        let mut body_bytes = Vec::new();
        value
            .header
            .zkp_body_serialize_content(&mut body_bytes)
            .map_err(|_| SynthesisError::AssignmentMissing)?;

        let pk_tree_root =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &validators.hash::<Blake2sHash>().0)?;
        let body_bytes = UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &body_bytes[32..])?;

        // Signature
        let signer_bitmap = BitVec::<MNT6Fq>::new_witness_vec(cs.clone(), &signer_bitmap[..])?;
        let signer_bitmap = signer_bitmap.0;

        let signature = G1Var::new_witness(cs.clone(), || Ok(signature))?;
        let round = UInt32::<MNT6Fq>::new_witness(cs, || Ok(justification_round))?;

        Ok(MacroBlockGadget {
            network,
            version,
            block_number,
            intermediate_header_bytes,
            history_root,
            pk_tree_root,
            body_bytes,
            signature,
            signer_bitmap,
            round,
            header_hash: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt6_753::{constraints::G2Var, Fq as MNT6Fq, G1Projective, G2Projective};
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::UniformRand;
    use nimiq_block::{MacroBody, MacroHeader, TendermintProof};
    use nimiq_bls::{KeyPair as BlsKeyPair, Signature};
    use nimiq_collections::bitset::BitSet;
    use nimiq_hash::{Blake2bHash, Hash};
    use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, SecureGenerate};
    use nimiq_primitives::{
        coin::Coin, networks::NetworkId, policy::Policy, slots_allocation::ValidatorsBuilder,
    };
    use nimiq_tendermint::ProposalMessage;
    use nimiq_test_log::test;
    use nimiq_test_utils::{block_production::TemporaryBlockProducer, test_rng::test_rng};
    use nimiq_transaction::reward::RewardTransaction;
    use rand::Rng;

    use super::*;

    fn random_macro_block() -> MacroBlock {
        let mut rng = test_rng(true);

        // Initialize validators.
        let mut validators = ValidatorsBuilder::new();
        let validator_address =
            Address::from_user_friendly_address("NQ05 563U 530Y XDRT L7GQ M6HE YRNU 20FE 4PNR")
                .unwrap();
        let bls_key_pair = BlsKeyPair::generate(&mut rng);
        let schnorr_key_pair = SchnorrKeyPair::generate(&mut rng);
        for _ in 0..Policy::SLOTS {
            validators.push(
                validator_address.clone(),
                bls_key_pair.public_key,
                schnorr_key_pair.public,
            );
        }

        let validators = Some(validators.build());

        // Initialize other body fields.
        let mut next_batch_initial_punished_set = BitSet::new();
        next_batch_initial_punished_set.insert(5);
        next_batch_initial_punished_set.insert(67);

        let transactions = vec![RewardTransaction {
            validator_address: Address::burn_address(),
            recipient: validator_address,
            value: Coin::from_u64_unchecked(12),
        }];

        let body = MacroBody { transactions };

        // Initialize the header.
        let body_root = body.hash();
        let header = MacroHeader {
            network: NetworkId::UnitAlbatross,
            version: 1,
            block_number: 5,
            round: 3,
            timestamp: 16274824,
            parent_hash: Blake2bHash(rng.gen()),
            parent_election_hash: Blake2bHash(rng.gen()),
            interlink: Some(vec![Blake2bHash(rng.gen()), Blake2bHash(rng.gen())]),
            seed: Default::default(),
            extra_data: vec![0x42],
            state_root: Blake2bHash(rng.gen()),
            body_root,
            diff_root: Blake2bHash(rng.gen()),
            history_root: Blake2bHash(rng.gen()),
            validators,
            next_batch_initial_punished_set: Default::default(),
        };

        MacroBlock {
            header,
            body: Some(body),
            justification: Some(TendermintProof {
                round: 2,
                sig: Default::default(),
            }),
        }
    }

    #[test]
    fn block_hash_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();
        let block = MacroBlock::non_empty_default();

        // Calculate hash using the primitive version.
        let primitive_hash = block.hash_blake2s();

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        // Calculate hash using the gadget version.
        let gadget_hash = block_var.hash(cs.clone()).unwrap();

        assert_eq!(primitive_hash.0, &gadget_hash.value().unwrap()[..]);

        let num_instance_vars = cs.num_instance_variables();
        let num_witness_vars = cs.num_witness_variables();

        // Test with a second block.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();
        let block = random_macro_block();

        // Calculate hash using the primitive version.
        let primitive_hash = block.hash_blake2s();

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

        // Calculate hash using the gadget version.
        let gadget_hash = block_var.hash(cs.clone()).unwrap();

        assert_eq!(primitive_hash.0, &gadget_hash.value().unwrap()[..]);

        assert_eq!(
            num_instance_vars,
            cs.num_instance_variables(),
            "Number of allocated public variables should not change"
        );
        assert_eq!(
            num_witness_vars,
            cs.num_witness_variables(),
            "Number of allocated private variables should not change"
        );
    }

    #[test]
    fn block_verify() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng(true);

        // Create more block parameters.
        let block_number = u32::rand(rng);
        let round = u32::rand(rng);

        // Create macro block with correct signers set.
        let mut block = MacroBlock::non_empty_default();
        block.header.network = NetworkId::UnitAlbatross;
        block.header.block_number = block_number;
        block.header.round = round;

        let (block, agg_pk) = TemporaryBlockProducer::finalize_macro_block(
            ProposalMessage {
                round,
                valid_round: None,
                proposal: block.header.clone(),
            },
            block.body.clone().unwrap(),
            block.hash_blake2s(),
        );

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();
        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk.0.public_key)).unwrap();

        // Verify block.
        assert!(block_var
            .verify_signature(cs, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_block_number() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng(true);

        // Create more block parameters.
        let block_number = u32::rand(rng);
        let round = u32::rand(rng);

        // Create macro block with correct signers set.
        let mut block = MacroBlock::non_empty_default();
        block.header.block_number = block_number;
        block.header.round = round;

        let (mut block, agg_pk) = TemporaryBlockProducer::finalize_macro_block(
            ProposalMessage {
                round,
                valid_round: None,
                proposal: block.header.clone(),
            },
            block.body.clone().unwrap(),
            block.hash_blake2s(),
        );

        // Create wrong block number.
        block.header.block_number += 1;

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();
        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk.0.public_key)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify_signature(cs, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_round_number() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng(true);

        // Create more block parameters.
        let block_number = u32::rand(rng);
        let round = u32::rand(rng);

        // Create macro block with correct signers set.
        let mut block = MacroBlock::non_empty_default();
        block.header.block_number = block_number;
        block.header.round = round;

        let (mut block, agg_pk) = TemporaryBlockProducer::finalize_macro_block(
            ProposalMessage {
                round,
                valid_round: None,
                proposal: block.header.clone(),
            },
            block.body.clone().unwrap(),
            block.hash_blake2s(),
        );

        // Create wrong round number.
        block.header.round += 1;

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();
        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk.0.public_key)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify_signature(cs, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_agg_pk() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng(true);

        // Create more block parameters.
        let block_number = u32::rand(rng);
        let round = u32::rand(rng);

        // Create macro block with correct signers set.
        let mut block = MacroBlock::non_empty_default();
        block.header.block_number = block_number;
        block.header.round = round;

        let (block, _agg_pk) = TemporaryBlockProducer::finalize_macro_block(
            ProposalMessage {
                round,
                valid_round: None,
                proposal: block.header.clone(),
            },
            block.body.clone().unwrap(),
            block.hash_blake2s(),
        );

        // Create wrong agg pk.
        let agg_pk = G2Projective::rand(rng);

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();
        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify_signature(cs, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_wrong_signature() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng(true);

        // Create more block parameters.
        let block_number = u32::rand(rng);
        let round = u32::rand(rng);

        // Create macro block with correct signers set.
        let mut block = MacroBlock::non_empty_default();
        block.header.block_number = block_number;
        block.header.round = round;

        let (mut block, agg_pk) = TemporaryBlockProducer::finalize_macro_block(
            ProposalMessage {
                round,
                valid_round: None,
                proposal: block.header.clone(),
            },
            block.body.clone().unwrap(),
            block.hash_blake2s(),
        );

        // Create wrong signature.
        block
            .justification
            .as_mut()
            .unwrap()
            .sig
            .signature
            .aggregate(&Signature::from(G1Projective::rand(rng)));

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();
        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk.0.public_key)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify_signature(cs, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }

    #[test]
    fn block_verify_too_few_signers() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng(true);

        // Create more block parameters.
        let block_number = u32::rand(rng);
        let round = u32::rand(rng);

        // Create macro block with too few signers.
        let mut block = MacroBlock::non_empty_default();
        block.header.block_number = block_number;
        block.header.round = round;

        let (mut block, agg_pk) = TemporaryBlockProducer::finalize_macro_block(
            ProposalMessage {
                round,
                valid_round: None,
                proposal: block.header.clone(),
            },
            block.body.clone().unwrap(),
            block.hash_blake2s(),
        );

        // Create wrong number of signers.
        block.justification.as_mut().unwrap().sig.signers = Default::default();

        // Allocate parameters in the circuit.
        let mut block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();
        let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk.0.public_key)).unwrap();

        // Verify block.
        assert!(!block_var
            .verify_signature(cs, &agg_pk_var)
            .unwrap()
            .value()
            .unwrap());
    }
}
