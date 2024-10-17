use std::path::Path;

use nimiq_block::{
    Block, BlockError, DoubleProposalProof, DoubleVoteProof, EquivocationProofError, ForkProof,
};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{
    AbstractBlockchain, PushError,
    PushError::{InvalidBlock, InvalidEquivocationProof},
    PushResult,
};
use nimiq_bls::AggregateSignature;
use nimiq_hash::{Blake2bHash, Blake2sHash, HashOutput};
use nimiq_keys::KeyPair;
use nimiq_primitives::{
    key_nibbles::KeyNibbles, networks::NetworkId, policy::Policy, TendermintIdentifier,
    TendermintProposal, TendermintStep,
};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::validator_address,
    test_custom_block::{next_macro_block, next_micro_block, next_skip_block, BlockConfig},
    test_rng::test_rng,
    zkp_test_data::{get_base_seed, simulate_merger_wrapper, ZKP_TEST_KEYS_PATH},
};
use nimiq_utils::key_rng::SecureGenerate;
use nimiq_vrf::VrfSeed;
use nimiq_zkp::ZKP_VERIFYING_DATA;

pub fn expect_push_micro_block(config: BlockConfig, expected_res: Result<PushResult, PushError>) {
    if config.test_micro {
        push_micro_after_macro(&config, &expected_res);
        push_micro_after_micro(&config, &expected_res);
        push_simple_skip_block(&config, &expected_res);
        push_rebranch(&config, &expected_res);
        push_rebranch_across_epochs(&config);
        push_fork(&config, &expected_res);
        push_rebranch_fork(&config, &expected_res);
    }

    if config.test_macro {
        simply_push_macro_block(&config, &expected_res);
    }

    if config.test_election {
        simply_push_election_block(&config, &expected_res);
    }
}

fn push_micro_after_macro(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    let temp_producer = TemporaryBlockProducer::new();

    let micro_block = {
        let blockchain = &temp_producer.blockchain.read();
        next_micro_block(&temp_producer.producer.signing_key, blockchain, config)
    };

    assert_eq!(&temp_producer.push(Block::Micro(micro_block)), expected_res);
}

fn push_micro_after_micro(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    let temp_producer = TemporaryBlockProducer::new();
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
    let temp_producer1 = TemporaryBlockProducer::new();

    temp_producer1.next_block(vec![], false);
    temp_producer1.next_block(vec![], true);

    let block = {
        let blockchain = &temp_producer1.blockchain.read();
        next_micro_block(&temp_producer1.producer.signing_key, blockchain, config)
    };

    assert_eq!(&temp_producer1.push(Block::Micro(block)), expected_res);
}

fn push_rebranch(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    // (Numbers denote accumulated skip blocks)
    // [0] - [0]
    //    \- [1]
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    let block = temp_producer1.next_block(vec![], false);
    assert_eq!(temp_producer2.push(block), Ok(PushResult::Extended));

    let block_1a = temp_producer1.next_block(vec![], false);

    let block_2a = {
        let blockchain = &temp_producer2.blockchain.read();
        next_skip_block(&temp_producer2.producer.voting_key, blockchain, config)
    };

    assert_eq!(temp_producer2.push(block_1a), Ok(PushResult::Extended));

    let expected_result = match &expected_res {
        // Skip blocks carry over the seed of the previous block. An incorrect seed if it matches
        // the previous block seed would fail as an invalid skip block proof
        Err(InvalidBlock(BlockError::InvalidSeed)) => {
            &Err(InvalidBlock(BlockError::InvalidSkipBlockProof))
        }
        Err(InvalidEquivocationProof(_)) => &Ok(PushResult::Rebranched),
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
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

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
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

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
fn push_rebranch_across_epochs(config: &BlockConfig) {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

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
        next_micro_block(&temp_producer2.producer.signing_key, blockchain, config)
    };

    // Pushing a block from a previous batch/epoch is atm caught before checking if it's a fork or known block
    assert_eq!(
        temp_producer1.push(Block::Micro(fork)),
        Ok(PushResult::Ignored)
    );
}

fn simply_push_macro_block(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    let temp_producer = TemporaryBlockProducer::new();

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

fn simply_push_election_block(config: &BlockConfig, expected_res: &Result<PushResult, PushError>) {
    let temp_producer = TemporaryBlockProducer::new();

    for _ in 0..Policy::blocks_per_epoch() - 1 {
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
    push_rebranch(&config, &Ok(PushResult::Rebranched));

    // Check that it doesn't rebranch across epochs
    push_rebranch_across_epochs(&config);

    // Check that it accepts forks as fork
    push_fork(&config, &Ok(PushResult::Forked));

    // Check that it rebranches to the better fork
    push_rebranch_fork(&config, &Ok(PushResult::Rebranched));

    // A simple push of a macro block
    simply_push_macro_block(&config, &Ok(PushResult::Extended));
}

#[test]
fn it_validates_network() {
    expect_push_micro_block(
        BlockConfig {
            network: Some(NetworkId::Main),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::NetworkMismatch)),
    );
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
fn it_validates_body_hash() {
    expect_push_micro_block(
        BlockConfig {
            body_hash: Some(Blake2sHash::default()),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::BodyHashMismatch)),
    );

    expect_push_micro_block(
        BlockConfig {
            missing_body: true,
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::MissingBody)),
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
fn it_validates_state_root() {
    let config = BlockConfig {
        state_root: Some(Blake2bHash::default()),
        ..Default::default()
    };

    // This does not fail since now the state root is properly calculated from strach
    push_micro_after_micro(&config, &Ok(PushResult::Extended));

    push_rebranch(&config, &Err(PushError::InvalidFork));

    push_rebranch_across_epochs(&config);
}

#[test]
fn it_validates_history_root() {
    let config = BlockConfig {
        history_root: Some(Blake2bHash::default()),
        ..Default::default()
    };
    push_micro_after_micro(&config, &Err(InvalidBlock(BlockError::InvalidHistoryRoot)));

    push_rebranch(&config, &Err(PushError::InvalidFork));

    push_rebranch_across_epochs(&config);
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
fn it_validates_interlink() {
    expect_push_micro_block(
        BlockConfig {
            test_micro: false,
            test_macro: false,
            interlink: Some(None),
            ..Default::default()
        },
        Err(InvalidBlock(BlockError::InvalidInterlink)),
    );
}

#[test]
fn it_validates_fork_proofs() {
    let mut rng = test_rng(true);

    let signing_key = KeyPair::generate(&mut rng);

    let header1 = TemporaryBlockProducer::new()
        .next_block(vec![], false)
        .unwrap_micro()
        .header;
    let mut header2 = header1.clone();
    header2.timestamp += 1;
    let header1_hash: Blake2bHash = header1.hash();
    let header2_hash: Blake2bHash = header2.hash();
    let justification1 = signing_key.sign(header1_hash.as_bytes());
    let justification2 = signing_key.sign(header2_hash.as_bytes());

    expect_push_micro_block(
        BlockConfig {
            equivocation_proofs: vec![ForkProof::new(
                validator_address(),
                header1,
                justification1,
                header2,
                justification2,
            )
            .into()],
            test_macro: false,
            test_election: false,
            ..Default::default()
        },
        Err(InvalidEquivocationProof(
            EquivocationProofError::InvalidJustification,
        )),
    )
}

#[test]
fn it_validates_double_proposal_proofs() {
    let mut rng = test_rng(true);

    let signing_key = KeyPair::generate(&mut rng);

    let temp_producer = TemporaryBlockProducer::new();
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }
    let header1 = temp_producer
        .next_block(vec![], false)
        .unwrap_macro()
        .header;
    let mut header2 = header1.clone();
    header2.timestamp += 1;
    let round = 0;
    let valid_round = None;
    let justification1 = signing_key.sign(
        TendermintProposal {
            proposal: &header1,
            round,
            valid_round,
        }
        .hash()
        .as_bytes(),
    );
    let justification2 = signing_key.sign(
        TendermintProposal {
            proposal: &header2,
            round,
            valid_round,
        }
        .hash()
        .as_bytes(),
    );

    expect_push_micro_block(
        BlockConfig {
            equivocation_proofs: vec![DoubleProposalProof::new(
                validator_address(),
                header1,
                round,
                valid_round,
                justification1,
                header2,
                round,
                valid_round,
                justification2,
            )
            .into()],
            test_macro: false,
            test_election: false,
            ..Default::default()
        },
        Err(InvalidEquivocationProof(
            EquivocationProofError::InvalidJustification,
        )),
    )
}

#[test]
fn it_validates_double_vote_proofs() {
    let temp_producer = TemporaryBlockProducer::new();
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }
    let macro_header = temp_producer
        .next_block(vec![], false)
        .unwrap_macro()
        .header;

    let validators = temp_producer
        .blockchain
        .read()
        .get_validators_for_epoch(Policy::epoch_at(macro_header.block_number), None)
        .unwrap();
    let validator = validators.validators[0].clone();

    expect_push_micro_block(
        BlockConfig {
            equivocation_proofs: vec![DoubleVoteProof::new(
                TendermintIdentifier {
                    network: macro_header.network,
                    block_number: macro_header.block_number,
                    round_number: 0,
                    step: TendermintStep::PreVote,
                },
                validator.address,
                None,
                AggregateSignature::new(),
                validator.slots.clone().map(|i| i.into()).collect(),
                Some(Blake2sHash::default()),
                AggregateSignature::new(),
                validator.slots.clone().map(|i| i.into()).collect(),
            )
            .into()],
            test_macro: false,
            test_election: false,
            ..Default::default()
        },
        Err(InvalidEquivocationProof(
            EquivocationProofError::InvalidJustification,
        )),
    )
}

#[test]
fn can_push_zkps() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Produce a full epoch of blocks.
    for _ in 0..Policy::blocks_per_epoch() - 1 {
        temp_producer1.next_block(vec![], false);
    }
    let election_block = temp_producer1.next_block(vec![], false);
    let block_number = election_block.block_number();

    // Try pushing an invalid ZKP.
    let invalid_zkp_proof = Default::default();
    let blockchain2 = temp_producer2.blockchain.upgradable_read();
    let result = Blockchain::push_zkp(
        blockchain2,
        election_block.clone(),
        invalid_zkp_proof,
        false,
    );

    assert_eq!(result, Err(PushError::InvalidZKP));
    {
        let blockchain2_rg = temp_producer2.blockchain.read();
        assert_eq!(
            blockchain2_rg.block_number(),
            Policy::genesis_block_number()
        );

        assert!(blockchain2_rg.can_enforce_validity_window());
        assert_eq!(blockchain2_rg.get_missing_accounts_range(None), None);
    }

    // Create and push a valid ZKP.
    let zkp_proof = simulate_merger_wrapper(
        Path::new(ZKP_TEST_KEYS_PATH),
        &temp_producer1.blockchain,
        &ZKP_VERIFYING_DATA,
        &mut get_base_seed(),
    );

    let blockchain2 = temp_producer2.blockchain.upgradable_read();
    let result = Blockchain::push_zkp(blockchain2, election_block, zkp_proof.proof.unwrap(), false);

    assert_eq!(result, Ok(PushResult::Extended));
    {
        let blockchain2_rg = temp_producer2.blockchain.read();
        assert_eq!(blockchain2_rg.block_number(), block_number);

        assert!(!blockchain2_rg.can_enforce_validity_window());
        assert_eq!(
            blockchain2_rg.get_missing_accounts_range(None),
            Some(KeyNibbles::ROOT..)
        );
    }
}
