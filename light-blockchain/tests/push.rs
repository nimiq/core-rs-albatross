use std::{path::Path, sync::Arc};

use nimiq_block::{Block, BlockError, SkipBlockProof};
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_blockchain_interface::{
    AbstractBlockchain, PushError, PushError::InvalidBlock, PushResult,
};
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    test_custom_block::{next_macro_block, next_micro_block, next_skip_block, BlockConfig},
    zkp_test_data::{get_base_seed, simulate_merger_wrapper, ZKP_TEST_KEYS_PATH},
};
use nimiq_vrf::VrfSeed;
use nimiq_zkp::ZKP_VERIFYING_DATA;
use parking_lot::RwLock;

fn remove_micro_body(block: Block) -> Block {
    match block {
        Block::Macro(macro_block) => Block::Macro(macro_block),
        Block::Micro(mut micro_block) => {
            micro_block.body = None;
            Block::Micro(micro_block)
        }
    }
}

/// This is just a wrapper around the TemporaryBlockProducer to incorporate the light blockchain
/// The regular blockchain takes care of producing blocks
pub struct TemporaryLightBlockProducer {
    pub temp_producer: TemporaryBlockProducer,
    pub light_blockchain: Arc<RwLock<LightBlockchain>>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub producer: BlockProducer,
}

impl Default for TemporaryLightBlockProducer {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporaryLightBlockProducer {
    pub fn new() -> Self {
        let light_blockchain =
            Arc::new(RwLock::new(LightBlockchain::new(NetworkId::UnitAlbatross)));

        let temp_producer = TemporaryBlockProducer::new();
        let blockchain = Arc::clone(&temp_producer.blockchain);
        let producer = temp_producer.producer.clone();
        TemporaryLightBlockProducer {
            temp_producer,
            light_blockchain,
            blockchain,
            producer,
        }
    }

    pub fn push(&self, block: Block) -> Result<PushResult, PushError> {
        let regular_result = self.temp_producer.push(block.clone());

        let light_result = LightBlockchain::push(
            self.light_blockchain.upgradable_read(),
            remove_micro_body(block),
        );

        // We expect both results to be equal:
        assert_eq!(regular_result, light_result);

        regular_result
    }

    pub fn next_block(&self, extra_data: Vec<u8>, skip_block: bool) -> Block {
        let block = self.next_block_no_push(extra_data, skip_block);
        assert_eq!(self.push(block.clone()), Ok(PushResult::Extended));
        block
    }

    pub fn next_block_no_push(&self, extra_data: Vec<u8>, skip_block: bool) -> Block {
        self.temp_producer
            .next_block_no_push(extra_data, skip_block)
    }

    pub fn create_skip_block_proof(&self) -> SkipBlockProof {
        self.temp_producer.create_skip_block_proof()
    }
}

pub fn expect_push_micro_block(config: BlockConfig, expected_res: Result<PushResult, PushError>) {
    if config.test_micro {
        push_micro_after_macro(&config, &expected_res);
        push_micro_after_micro(&config, &expected_res);
        push_simple_skip_block(&config, &expected_res);
        push_rebranch(config.clone(), &expected_res);
        push_rebranch_across_epochs(config.clone());
        push_fork(&config, &expected_res);
        push_rebranch_fork(&config, &expected_res);
    }

    if config.test_macro {
        simply_push_macro_block(&config, &expected_res);
    }
}

fn push_micro_after_macro(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    let temp_producer = TemporaryLightBlockProducer::new();

    let micro_block = {
        let blockchain = &temp_producer.blockchain.read();
        next_micro_block(&temp_producer.producer.signing_key, blockchain, config)
    };

    assert_eq!(&temp_producer.push(Block::Micro(micro_block)), expected_res);
}

fn push_micro_after_micro(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    let temp_producer = TemporaryLightBlockProducer::new();
    temp_producer.next_block(vec![], false);

    let micro_block = {
        let blockchain = &temp_producer.blockchain.read();
        next_micro_block(&temp_producer.producer.signing_key, blockchain, config)
    };

    assert_eq!(&temp_producer.push(Block::Micro(micro_block)), expected_res);
}

fn push_simple_skip_block(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    // (Numbers denote accumulated skip blocks)
    // [0] - [0]
    //    \- [1] - [1]
    let temp_producer1 = TemporaryLightBlockProducer::new();

    temp_producer1.next_block(vec![], false);
    temp_producer1.next_block(vec![], true);

    let block = {
        let blockchain = &temp_producer1.blockchain.read();
        next_micro_block(&temp_producer1.producer.signing_key, blockchain, config)
    };

    assert_eq!(&temp_producer1.push(Block::Micro(block)), expected_res);
}

fn push_rebranch(config: BlockConfig, expected_res: &Result<PushResult, PushError>) {
    // (Numbers denote accumulated skip blocks)
    // [0] - [0]
    //    \- [1]
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    let block = temp_producer1.next_block(vec![], false);
    assert_eq!(temp_producer2.push(block), Ok(PushResult::Extended));

    let block_1a = temp_producer1.next_block(vec![], false);

    let block_2a = {
        let blockchain = &temp_producer2.blockchain.read();
        next_skip_block(&temp_producer2.producer.voting_key, blockchain, &config)
    };

    assert_eq!(temp_producer2.push(block_1a), Ok(PushResult::Extended));

    let expected_result = match &expected_res {
        // Skip blocks carry over the seed of the previous block. An incorrect seed if it matches
        // the previous block seed would fail as an invalid skip block proof
        Err(InvalidBlock(BlockError::InvalidSeed)) => {
            &Err(InvalidBlock(BlockError::InvalidSkipBlockProof))
        }
        _ => expected_res,
    };

    assert_eq!(
        &temp_producer1.push(Block::Micro(block_2a)),
        expected_result
    );
}

fn push_fork(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    // (Numbers denote accumulated skip blocks)
    // [0] - [0]
    //    \- [0]
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    let block = temp_producer1.next_block(vec![], false);
    assert_eq!(temp_producer2.push(block), Ok(PushResult::Extended));

    let block_1a = temp_producer1.next_block(vec![], false);

    let block_2a = {
        let blockchain = &temp_producer2.blockchain.read();
        next_micro_block(&temp_producer2.producer.signing_key, blockchain, config)
    };

    assert_eq!(temp_producer2.push(block_1a), Ok(PushResult::Extended));

    assert_eq!(&temp_producer1.push(Block::Micro(block_2a)), expected_res);
}

fn push_rebranch_fork(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    // Build forks using two producers.
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    // Case 1: easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [0]
    let block = temp_producer1.next_block(vec![], false);
    assert_eq!(temp_producer2.push(block), Ok(PushResult::Extended));

    let fork1 = temp_producer1.next_block(vec![0x48], false);
    let fork2 = temp_producer2.next_block(vec![], false);

    // Check that each one accepts other fork.
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Forked));

    let better = {
        let blockchain = &temp_producer1.blockchain.read();
        next_micro_block(&temp_producer1.producer.signing_key, blockchain, config)
    };

    // Check that producer 2 rebranches.
    assert_eq!(&temp_producer2.push(Block::Micro(better)), expected_res);
}

/// Check that it doesn't rebranch across epochs. This push should always result in OK::Ignored.
fn push_rebranch_across_epochs(config: BlockConfig) {
    // Build forks using two producers.
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    // The number in [_] represents the epoch number
    //              a
    // [0] - [0] - [0]
    //          \- [0] - ... - [0] - [1]

    let ancestor = temp_producer1.next_block(vec![], false);
    assert_eq!(temp_producer2.push(ancestor), Ok(PushResult::Extended));

    // progress the chain across an epoch boundary.
    for _ in 0..Policy::blocks_per_epoch() {
        temp_producer1.next_block(vec![], false);
    }

    let fork = {
        let blockchain = &temp_producer2.blockchain.read();
        next_micro_block(&temp_producer2.producer.signing_key, blockchain, &config)
    };

    // Pushing a block from a previous batch/epoch is atm caught before checking if it's a fork or known block
    assert_eq!(
        temp_producer1.push(Block::Micro(fork)),
        Ok(PushResult::Ignored)
    );
}

fn simply_push_macro_block(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    let temp_producer = TemporaryLightBlockProducer::new();

    for _ in 0..Policy::blocks_per_batch() - 1 {
        let block = temp_producer.next_block(vec![], false);
        temp_producer.push(block.clone()).unwrap();
    }

    let block = {
        let blockchain = temp_producer.blockchain.read();
        next_macro_block(
            &temp_producer.producer.signing_key,
            &temp_producer.producer.voting_key,
            &blockchain,
            config,
        )
    };

    assert_eq!(&temp_producer.push(block), expected_res);
}

#[test]
fn it_works_with_valid_blocks() {
    let config = BlockConfig::default();

    // Just a simple push onto a normal chain
    push_micro_after_micro(&config, &Ok(PushResult::Extended));

    // Check the normal behaviour for a rebranch
    push_rebranch(BlockConfig::default(), &Ok(PushResult::Rebranched));

    // Check that it doesn't rebranch across epochs
    push_rebranch_across_epochs(config.clone());

    // Check that it accepts forks as fork
    push_fork(&config, &Ok(PushResult::Forked));

    // Check that it rebranches to the better fork
    push_rebranch_fork(&config, &Ok(PushResult::Rebranched));

    // A simple push of a macro block
    simply_push_macro_block(&config, &Ok(PushResult::Extended));
}

#[test]
fn it_validates_version() {
    expect_push_micro_block(
        BlockConfig {
            version: Some(Policy::VERSION - 1),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::UnsupportedVersion)),
    );
}

#[test]
fn it_validates_extra_data() {
    expect_push_micro_block(
        BlockConfig {
            extra_data: [0u8; 33].into(),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::ExtraDataTooLarge)),
    );
}

#[test]
fn it_validates_parent_hash() {
    expect_push_micro_block(
        BlockConfig {
            parent_hash: Some(Blake2bHash::default()),
            ..Default::default()
        },
        Err(PushError::Orphan),
    );
}

#[test]
fn it_validates_block_number() {
    expect_push_micro_block(
        BlockConfig {
            test_macro: false,
            test_election: false,
            block_number_offset: 1,
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::InvalidBlockNumber)),
    );
}

#[test]
fn it_validates_block_time() {
    expect_push_micro_block(
        BlockConfig {
            timestamp_offset: -2,
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::InvalidTimestamp)),
    );
}

#[test]
fn it_validates_seed() {
    expect_push_micro_block(
        BlockConfig {
            seed: Some(VrfSeed::default()),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::InvalidSeed)),
    );
}

#[test]
fn it_validates_parent_election_hash() {
    expect_push_micro_block(
        BlockConfig {
            test_micro: false,
            parent_election_hash: Some(Blake2bHash::default()),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::InvalidParentElectionHash)),
    );
}

#[test]
fn it_validates_tendermint_round_number() {
    expect_push_micro_block(
        BlockConfig {
            test_micro: false,
            tendermint_round: Some(3),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::InvalidJustification)),
    );
}

#[test]
fn it_can_rebranch_skip_block() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    // Case 1: easy rebranch (number denotes accumulated skip blocks)
    // [0] - [0] - [0] - [0]
    //          \- [1] - [1]
    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(vec![], false);
    let fork1 = temp_producer2.next_block(vec![], true);

    let inferior2 = temp_producer1.next_block(vec![], false);
    let fork2 = temp_producer2.next_block(vec![], false);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Ok(PushResult::Ignored));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));

    // Case 2: not obvious rebranch rebranch (number denotes accumulated skip block)
    // ... - [1] - [1] - [2]
    //          \- [2] - [2]
    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(vec![], false);
    let fork1 = temp_producer2.next_block(vec![], true);

    let inferior2 = temp_producer1.next_block(vec![], true);
    let fork2 = temp_producer2.next_block(vec![], false);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Ok(PushResult::Ignored));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));
}

#[test]
fn it_can_ignore_orphan_blocks() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(vec![], false);
    let fork1 = temp_producer2.next_block(vec![], true);

    let inferior2 = temp_producer1.next_block(vec![], false);
    let fork2 = temp_producer2.next_block(vec![], false);

    let inferior3 = temp_producer1.next_block(vec![], false);
    let fork3 = temp_producer2.next_block(vec![], false);

    // Try to push first inferior3, which at this point would be an orphan
    assert_eq!(temp_producer2.push(inferior3), Err(PushError::Orphan));
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Ok(PushResult::Ignored));

    // Try to push fork3, which should be orphan
    assert_eq!(temp_producer1.push(fork3.clone()), Err(PushError::Orphan));

    // Now check that we rebranch
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));
    assert_eq!(temp_producer1.push(fork3), Ok(PushResult::Extended));
}

#[test]
fn micro_block_works_after_macro_block() {
    let temp_producer = TemporaryLightBlockProducer::new();
    let genesis_block_number = Policy::genesis_block_number();

    // Apply an entire batch including macro block on skip_block/round_number zero
    for _ in 0..Policy::blocks_per_batch() {
        temp_producer.next_block(vec![], false);
    }
    // Make sure we are at the beginning of the batch and all block were applied
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + genesis_block_number
    );

    // Test if a micro block can be rebranched immediately after
    // a round_number 0 macro block

    // Create a couple of skip blocks
    let block = temp_producer.next_block_no_push(vec![], true);
    let rebranch = temp_producer.next_block_no_push(vec![], true);
    // Push first skip block
    temp_producer.push(block).unwrap();
    // Make sure this was an extend
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + 1 + genesis_block_number
    );
    // And rebranch it to block the chain with only one skip block
    temp_producer.push(rebranch).unwrap();
    // Make sure this was a rebranch
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + 1 + genesis_block_number
    );

    // Apply the rest of the batch including macro block on round_number one
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }
    // Make sure we are at the beginning of the batch
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() * 2 + genesis_block_number
    );

    // Test if a micro block can be rebranched immediately after
    // a round_number non 0 macro block

    // Create blocks for a chain with accumulated skip blocks after the batch of 0, 1 and 2
    let block = temp_producer.next_block_no_push(vec![], true);
    let rebranch1 = temp_producer.next_block_no_push(vec![], true);

    // Apply them each rebranching the previous one
    temp_producer.push(block).unwrap();
    temp_producer.push(rebranch1).unwrap();

    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() * 2 + 1 + genesis_block_number
    );
}

#[test]
fn it_can_rebranch_forks() {
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    // Case 2: more difficult rebranch
    //              a     b     c     d
    // [0] - [0] - [0] - [0] - [0] - [0]
    //          \- [0] - [0] - [1] - [1]
    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let fork1a = temp_producer1.next_block(vec![0x48], false);
    let fork2a = temp_producer2.next_block(vec![], false);

    let fork1b = temp_producer1.next_block(vec![], false);
    let fork2b = temp_producer2.next_block(vec![], false);

    let fork1c = temp_producer1.next_block(vec![], false);
    let fork2c = temp_producer2.next_block(vec![], true);

    let fork1d = temp_producer1.next_block(vec![], false);
    let fork2d = temp_producer2.next_block(vec![], false);

    // Check that each one accepts other fork.
    assert_eq!(temp_producer1.push(fork2a), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1a), Ok(PushResult::Forked));
    assert_eq!(temp_producer1.push(fork2b), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1b), Ok(PushResult::Forked));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork2c), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1c), Ok(PushResult::Ignored));

    assert_eq!(temp_producer1.push(fork2d), Ok(PushResult::Extended));
    assert_eq!(temp_producer2.push(fork1d), Ok(PushResult::Ignored));
}

#[test]
fn it_can_rebranch_at_macro_block() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryLightBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    // The numbers in [X/Y] represent block_number (X) and accumulated number of skip blocks (Y):
    //
    // [0/0] ... [1/0] - [1/0]
    //                \- [1/1]

    let mut block;
    loop {
        block = temp_producer1.next_block(vec![], false);
        temp_producer2.push(block.clone()).unwrap();
        if block.is_macro() {
            break;
        }
    }

    let fork1 = temp_producer1.next_block(vec![], false);
    let fork2 = temp_producer2.next_block(vec![], true);

    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Ignored));
}

#[test]
fn it_can_rebranch_to_inferior_macro_block() {
    // Build forks using two producers.
    let producer1 = TemporaryLightBlockProducer::new();
    let producer2 = TemporaryLightBlockProducer::new();

    // (1 denotes a skip block)
    // [0] - [0] - ... - [0] - [macro 0]
    //    \- [1] - ... - [0]

    // Do one iteration first to create fork
    let inferior = producer1.next_block(vec![], false);
    producer2.next_block(vec![], true);
    assert_eq!(producer2.push(inferior), Ok(PushResult::Ignored));

    // Complete a batch
    for _ in 1..Policy::blocks_per_batch() - 1 {
        let inferior = producer1.next_block(vec![], false);
        producer2.next_block(vec![], false);
        assert_eq!(producer2.push(inferior), Ok(PushResult::Ignored));
    }

    let macro_block = producer1.next_block(vec![], false);
    assert!(macro_block.is_macro());

    // Check that producer 2 rebranches.
    assert_eq!(producer2.push(macro_block), Ok(PushResult::Rebranched));

    // Push one additional block and check that producer 2 accepts it.
    let block = producer1.next_block(vec![], false);
    assert_eq!(producer2.push(block), Ok(PushResult::Extended));

    // Check that both chains are in an identical state.
    let blockchain1 = producer1.blockchain.read();
    let blockchain2 = producer2.blockchain.read();
    assert_eq!(blockchain1.state.head_hash, blockchain2.state.head_hash);
    assert_eq!(
        blockchain1.state.macro_head_hash,
        blockchain2.state.macro_head_hash
    );
    assert_eq!(
        blockchain1.state.election_head_hash,
        blockchain2.state.election_head_hash
    );
    assert_eq!(
        blockchain1.state.current_slots,
        blockchain2.state.current_slots
    );
    assert_eq!(
        blockchain1.state.previous_slots,
        blockchain2.state.previous_slots
    );
}

#[test]
fn can_push_zkps() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryLightBlockProducer::new();

    // Produce a full epoch of blocks.
    for _ in 0..Policy::blocks_per_epoch() - 1 {
        temp_producer1.next_block(vec![], false);
    }
    let election_block = temp_producer1.next_block(vec![], false);
    let block_number = election_block.block_number();

    // Try pushing an invalid ZKP.
    let invalid_zkp_proof = Default::default();
    let blockchain2 = temp_producer2.light_blockchain.upgradable_read();
    let result = LightBlockchain::push_zkp(
        blockchain2,
        election_block.clone(),
        invalid_zkp_proof,
        false,
    );

    assert_eq!(result, Err(PushError::InvalidZKP));
    {
        let blockchain2_rg = temp_producer2.light_blockchain.read();
        assert_eq!(
            blockchain2_rg.block_number(),
            Policy::genesis_block_number()
        );
    }

    // Create a valid ZKP.
    let zkp_proof = simulate_merger_wrapper(
        Path::new(ZKP_TEST_KEYS_PATH),
        &temp_producer1.blockchain,
        &ZKP_VERIFYING_DATA,
        &mut get_base_seed(),
    );

    // Push the valid ZKP.
    let blockchain2 = temp_producer2.light_blockchain.upgradable_read();
    let result =
        LightBlockchain::push_zkp(blockchain2, election_block, zkp_proof.proof.unwrap(), false);

    assert_eq!(result, Ok(PushResult::Extended));
    {
        let blockchain2_rg = temp_producer2.light_blockchain.read();
        assert_eq!(blockchain2_rg.block_number(), block_number);
    }
}
