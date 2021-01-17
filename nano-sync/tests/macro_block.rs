use ark_ec::ProjectiveCurve;
use ark_ff::Zero;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, G2Var};
use ark_mnt6_753::{Fr, G1Projective, G2Projective};
use ark_r1cs_std::prelude::{AllocVar, Boolean, UInt32};
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::ConstraintSystem;
use ark_std::ops::MulAssign;
use ark_std::{test_rng, UniformRand};
use rand::RngCore;

use nimiq_bls::utils::bytes_to_bits;
use nimiq_nano_sync::constants::MIN_SIGNERS;
use nimiq_nano_sync::gadgets::mnt4::MacroBlockGadget;
use nimiq_nano_sync::primitives::{pedersen_generators, MacroBlock};

#[test]
fn block_hash_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT4Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create block parameters.
    let block_number = u32::rand(rng);

    let round_number = u32::rand(rng);

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pk_tree_root = bytes.to_vec();

    let block = MacroBlock::default();

    // Calculate hash using the primitive version.
    let primitive_hash = block.hash(block_number, round_number, pk_tree_root.clone());

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Calculate hash using the gadget version.
    let gadget_hash = block_var
        .get_hash(
            &block_number_var,
            &round_number_var,
            &pk_tree_root_var,
            &generators_var,
        )
        .unwrap();

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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Create wrong block number.
    let block_number = u32::rand(rng);

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(!block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Create wrong round number.
    let round_number = u32::rand(rng);

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(!block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Create wrong public keys tree root.
    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pk_tree_root = bytes.to_vec();

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(!block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Create wrong agg pk.
    let agg_pk = G2Projective::rand(rng);

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(!block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Create wrong header hash.
    let mut header_hash = [0u8; 32];
    rng.fill_bytes(&mut header_hash);
    block.header_hash = header_hash;

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(!block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Create wrong signature.
    block.signature = G1Projective::rand(rng);

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(!block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
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
    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..MIN_SIGNERS - 1 {
        block.sign(sk, i, block_number, round_number, pk_tree_root.clone());
        agg_pk += &pk;
    }

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::new_witness(cs.clone(), || Ok(block)).unwrap();

    let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

    let round_number_var = UInt32::new_witness(cs.clone(), || Ok(round_number)).unwrap();

    let pk_tree_root_var =
        Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(bytes_to_bits(&pk_tree_root)))
            .unwrap();

    let agg_pk_var = G2Var::new_witness(cs.clone(), || Ok(agg_pk)).unwrap();

    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

    // Verify block.
    assert!(!block_var
        .verify(
            cs,
            &pk_tree_root_var,
            &block_number_var,
            &round_number_var,
            &agg_pk_var,
            &generators_var,
        )
        .unwrap()
        .value()
        .unwrap());
}
