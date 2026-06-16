//! Regression tests for the forged skip block `state_root` vulnerability.
//!
//! Validators only sign the reduced `SkipBlockInfo` (network id, block number, VRF entropy) of a
//! skip block, not its full header. Before `upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING` this let
//! a public attacker replay a valid skip block proof onto a header carrying a forged `state_root`.
//! An incomplete (state live syncing) node could not recompute the state root and adopted the
//! forged head, stalling further state synchronization.
//!
//! From the binding version on, the proof additionally commits to the `state_root`, so such a
//! replay invalidates the proof. The binding is gated by the block's protocol version (which cannot
//! be decreased, see `Block::verify_immediate_successor`) so historical proofs remain verifiable.

use nimiq_block::{Block, BlockError};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{policy::upgrades, slots_allocation::Validators};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    test_custom_block::{next_skip_block, BlockConfig},
};

/// A `state_root` that differs from any real (deterministic) skip block state root.
fn forged_state_root() -> Blake2bHash {
    Blake2bHash::from([0xABu8; 32])
}

/// Builds the next skip block on top of the (complete) producer's head with the given protocol
/// `version`. If `forged_state_root` is `Some`, it overrides the header's `state_root` to model an
/// attacker replaying an honest proof onto a forged header. Returns the block together with the
/// validators it must be verified against.
fn next_skip_block_for(
    producer: &TemporaryBlockProducer,
    version: u16,
    forged_state_root: Option<Blake2bHash>,
) -> (Block, Validators) {
    let blockchain = producer.blockchain.read();
    let config = BlockConfig {
        version: Some(version),
        // The proof is always signed over the real state root; this only rewrites the header.
        state_root: forged_state_root,
        ..Default::default()
    };
    let skip_block = next_skip_block(&producer.producer.voting_key, &blockchain, &config);
    let validators = blockchain.current_validators().unwrap().clone();
    (Block::Micro(skip_block), validators)
}

#[test]
fn it_accepts_a_correctly_bound_skip_block_proof() {
    let producer = TemporaryBlockProducer::new();

    // A genuine skip block at the binding version: the proof commits to the real state root, which
    // matches the header.
    let (block, validators) =
        next_skip_block_for(&producer, upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING, None);

    assert_eq!(block.verify_validators(&validators), Ok(()));
}

#[test]
fn it_rejects_a_forged_state_root_skip_block_proof() {
    let producer = TemporaryBlockProducer::new();

    // The same proof replayed onto a header with a forged state root. From the binding version on,
    // the forged state root is part of the verified message and no longer matches the signature.
    let (block, validators) = next_skip_block_for(
        &producer,
        upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING,
        Some(forged_state_root()),
    );

    assert_eq!(
        block.verify_validators(&validators),
        Err(BlockError::InvalidSkipBlockProof)
    );
}

#[test]
fn it_does_not_bind_state_root_before_the_upgrade_version() {
    let producer = TemporaryBlockProducer::new();

    // Below the binding version the state root is not part of the signed message, so a forged state
    // root still produces a valid proof. This is the pre-fork behaviour, kept so that historical
    // version 1 skip block proofs remain verifiable.
    let legacy_version = upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING - 1;
    let (block, validators) =
        next_skip_block_for(&producer, legacy_version, Some(forged_state_root()));

    assert_eq!(block.verify_validators(&validators), Ok(()));
}
