use std::ops::AddAssign;

use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fq, Fr, G1Projective, G2Projective};
use algebra_core::fields::Field;
use algebra_core::{test_rng, ProjectiveCurve, Zero};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, UInt32, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::constants::{sum_generator_g1_mnt6, MAX_NON_SIGNERS, MIN_SIGNERS, VALIDATOR_SLOTS};
use nano_sync::gadgets::mnt4::{MacroBlockGadget, Round};
use nano_sync::primitives::{pedersen_generators, MacroBlock};

// When running tests you are advised to run only one test at a time or you might run out of RAM.
// Also they take a long time to run. This is why they have the ignore flag.

#[derive(Clone)]
struct DummyCircuit {
    prepare_agg_pk: G2Projective,
    commit_agg_pk: G2Projective,
    block_number: u32,
    pks_commitment: Vec<u8>,
    block: MacroBlock,
}

impl DummyCircuit {
    pub fn new(
        prepare_agg_pk: G2Projective,
        commit_agg_pk: G2Projective,
        block_number: u32,
        pks_commitment: Vec<u8>,
        block: MacroBlock,
    ) -> Self {
        Self {
            prepare_agg_pk,
            commit_agg_pk,
            block_number,
            pks_commitment,
            block,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for DummyCircuit {
    fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let max_non_signers_var = FqGadget::alloc_constant(
            cs.ns(|| "alloc max non signers"),
            &Fq::from(MAX_NON_SIGNERS as u64),
        )?;

        let sig_generator_var = G2Gadget::alloc_constant(
            cs.ns(|| "alloc signature generator"),
            &G2Projective::prime_subgroup_generator(),
        )?;

        let sum_generator_g1_var =
            G1Gadget::alloc_constant(cs.ns(|| "alloc sum generator g1"), &sum_generator_g1_mnt6())?;

        let pedersen_generators_var = Vec::<G1Gadget>::alloc_constant(
            cs.ns(|| "alloc pedersen_generators"),
            pedersen_generators(256),
        )?;

        // Allocate private inputs
        let prepare_agg_pk_var = G2Gadget::alloc(cs.ns(|| "allocating prepare agg pk"), || {
            Ok(self.prepare_agg_pk)
        })?;

        let commit_agg_pk_var = G2Gadget::alloc(cs.ns(|| "allocating commit agg pk"), || {
            Ok(self.commit_agg_pk)
        })?;

        let block_number_var =
            UInt32::alloc(cs.ns(|| "alloc block number"), Some(self.block_number))?;

        let pks_commitment_var =
            UInt8::alloc_vec(cs.ns(|| "alloc pks commitment"), &self.pks_commitment)?;

        let block_var = MacroBlockGadget::alloc(cs.ns(|| "alloc macro block"), || Ok(&self.block))?;

        // Verify Macro block.
        block_var.verify(
            cs.ns(|| "verify macro block"),
            &pks_commitment_var,
            &block_number_var,
            &prepare_agg_pk_var,
            &commit_agg_pk_var,
            &max_non_signers_var,
            &sig_generator_var,
            &sum_generator_g1_var,
            &pedersen_generators_var,
        )
    }
}

#[test]
#[ignore]
fn hash_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create block parameters.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let block_number = 77;

    let block = MacroBlock::default();

    // Calculate hash using the primitive version.
    let primitive_out = block.hash(1, block_number, pks_commitment.clone());

    // Allocate primitive result to facilitate comparison.
    let primitive_out_var =
        G1Gadget::alloc(cs.ns(|| "allocate primitive result"), || Ok(primitive_out)).unwrap();

    // Allocate parameters in the circuit.
    let block_var = MacroBlockGadget::alloc(cs.ns(|| "alloc macro block"), || Ok(&block)).unwrap();

    let block_number_var =
        UInt32::alloc(cs.ns(|| "alloc block number"), Some(block_number)).unwrap();

    let pks_commitment_var =
        UInt8::alloc_vec(cs.ns(|| "alloc pks commitment"), &pks_commitment).unwrap();

    let sum_generator_g1_var =
        G1Gadget::alloc_constant(cs.ns(|| "alloc sum generator g1"), &sum_generator_g1_mnt6())
            .unwrap();

    let pedersen_generators_var = Vec::<G1Gadget>::alloc_constant(
        cs.ns(|| "alloc pedersen_generators"),
        pedersen_generators(256),
    )
    .unwrap();

    // Calculate hash using the gadget version.
    let gadget_out = block_var
        .get_hash(
            cs.ns(|| "get hash"),
            Round::Commit,
            &block_number_var,
            &pks_commitment_var,
            &sum_generator_g1_var,
            &pedersen_generators_var,
        )
        .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}

#[test]
#[ignore]
fn macro_block_works() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    if !test_cs.is_satisfied() {
        println!("Unsatisfied @ {}", test_cs.which_is_unsatisfied().unwrap());
        assert!(false);
    }
}

#[test]
#[ignore]
fn wrong_prepare_agg_pk() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        G2Projective::prime_subgroup_generator(),
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn wrong_commit_agg_pk() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        G2Projective::prime_subgroup_generator(),
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn wrong_block_number() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(prepare_agg_pk, commit_agg_pk, 88, pks_commitment, block);
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn wrong_pks_commitment() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        vec![0u8; 95],
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn wrong_header_hash() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Change the header hash to a wrong value.
    block.header_hash = [1; 32];

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn wrong_prepare_signature() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Change the prepare signature to a wrong value.
    block.prepare_signature = G1Projective::prime_subgroup_generator();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn wrong_commit_signature() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Change the commit signature to a wrong value.
    block.commit_signature = G1Projective::prime_subgroup_generator();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn too_few_signers_prepare() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets. But too few signers in the prepare
    // round.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS - 1 {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn too_few_signers_commit() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets. But too few signers in the commit
    // round.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in 0..MIN_SIGNERS - 1 {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
#[ignore]
fn mismatched_signer_sets() {
    // Create random keys.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let sk = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(sk);

    // Create more block parameters.
    let block_number = 77;

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let pks_commitment = bytes.to_vec();

    let mut prepare_agg_pk = G2Projective::zero();
    let mut commit_agg_pk = G2Projective::zero();

    // Create macro block with correct prepare and commit sets. But with mismatched prepare and
    // commit signer sets. Note that not enough signers signed both the prepare and commit rounds.
    let mut block = MacroBlock::without_signatures([0; 32]);

    for i in 0..MIN_SIGNERS {
        block.sign_prepare(sk, i, block_number, pks_commitment.clone());
        prepare_agg_pk.add_assign(&pk);
    }

    for i in MAX_NON_SIGNERS..VALIDATOR_SLOTS {
        block.sign_commit(sk, i, block_number, pks_commitment.clone());
        commit_agg_pk.add_assign(&pk);
    }

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = DummyCircuit::new(
        prepare_agg_pk,
        commit_agg_pk,
        block_number,
        pks_commitment,
        block,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}
