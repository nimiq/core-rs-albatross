use std::sync::Arc;

use nimiq_block::{Block, BlockError, SkipBlockProof};
use nimiq_block_production::{
    test_custom_block::{next_macro_block, next_micro_block, next_skip_block, BlockConfig},
    BlockProducer,
};
use nimiq_blockchain::{Blockchain, PushError, PushError::InvalidBlock, PushResult};
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::block_production::TemporaryBlockProducer;
use nimiq_vrf::VrfSeed;
use parking_lot::RwLock;

fn remove_micro_body(block: Block) -> Block {
    let block = match block {
        Block::Macro(macro_block) => Block::Macro(macro_block),
        Block::Micro(mut micro_block) => {
            micro_block.body = None;
            Block::Micro(micro_block)
        }
    };
    block
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
    if !config.macro_only {
        push_micro_after_macro(&config, &expected_res);
        push_micro_after_micro(&config, &expected_res);
        push_simple_skip_block(&config, &expected_res);
        push_rebranch(config.clone(), &expected_res);
        push_rebranch_across_epochs(config.clone());
        push_fork(&config, &expected_res);
        push_rebranch_fork(&config, &expected_res);
    }

    if !config.micro_only {
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
        Err(PushError::InvalidBlock(BlockError::InvalidSeed)) => {
            &Err(PushError::InvalidBlock(BlockError::InvalidSkipBlockProof))
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
        next_micro_block(&&&temp_producer2.producer.signing_key, blockchain, config)
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
        next_micro_block(&&&temp_producer1.producer.signing_key, blockchain, config)
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

    // Pushing a block from a previous batch/epoch is atm cought before checking if it's a fork or known block
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

    assert_eq!(&temp_producer.push(block.clone()), expected_res);
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
            micro_only: true,
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
            macro_only: true,
            parent_election_hash: Some(Blake2bHash::default()),
            ..Default::default()
        },
        Err(PushError::InvalidBlock(
            BlockError::InvalidParentElectionHash,
        )),
    );
}

#[test]
fn it_validates_tendermint_round_number() {
    expect_push_micro_block(
        BlockConfig {
            macro_only: true,
            tendermint_round: Some(3),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::InvalidJustification)),
    );
}

#[test]
fn it_can_rebranch_skip_block() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Case 1: easy rebranch (number denotes accumulated skip blocks)
    // [0] - [0] - [0] - [0]
    //          \- [1] - [1]
    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(vec![], false);
    temp_producer2.next_block(vec![], true);

    temp_producer1.next_block(vec![], false);
    temp_producer2.next_block(vec![], false);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
}
