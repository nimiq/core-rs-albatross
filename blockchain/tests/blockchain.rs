use std::sync::Arc;

use nimiq_block::{Block, BlockError};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::policy::{upgrades, Policy};
use nimiq_tendermint::ProposalMessage;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    test_custom_block::{finalize_macro_block, next_macro_block_proposal},
    versions::{assert_all_versions, bp},
};

fn produce_blocks(temp_producer: &TemporaryBlockProducer, count: u32) {
    for _ in 0..count {
        temp_producer.next_block(vec![], false);
    }
}

#[test]
fn prune_epoch_micro_blocks() {
    // Goal: test that every MicroBlock at a given height is removed when prune_epoch is executed.

    let temp_producer = TemporaryBlockProducer::new();
    let blockchain = Arc::clone(&temp_producer.blockchain);
    let producer = temp_producer.producer;

    // Create different MicroBlocks, push them, and then check they do exist.
    // We ensure more than one MicroBlock at same height exists.
    let micro_block1 = {
        let bc_read = blockchain.read();
        producer
            .next_micro_block(
                &bc_read,
                &bc_read.time.now() + 1000,
                vec![],
                vec![],
                vec![0x42],
                None,
            )
            .unwrap()
    };
    let micro_block2 = {
        let bc_read = blockchain.read();
        producer
            .next_micro_block(
                &bc_read,
                bc_read.time.now() + 100,
                vec![],
                vec![],
                vec![0x32],
                None,
            )
            .unwrap()
    };
    let micro_block3 = {
        let bc_read = blockchain.read();
        producer
            .next_micro_block(
                &bc_read,
                bc_read.time.now() + 10000,
                vec![],
                vec![],
                vec![0x82],
                None,
            )
            .unwrap()
    };

    assert_eq!(
        Blockchain::push(
            blockchain.upgradable_read(),
            Block::Micro(micro_block1.clone())
        ),
        Ok(PushResult::Extended)
    );
    assert_eq!(
        Blockchain::push(
            blockchain.upgradable_read(),
            Block::Micro(micro_block2.clone())
        ),
        Ok(PushResult::Forked)
    );
    assert_eq!(
        Blockchain::push(
            blockchain.upgradable_read(),
            Block::Micro(micro_block3.clone())
        ),
        Ok(PushResult::Forked)
    );

    let bc_read = blockchain.read();
    assert!(bc_read
        .chain_store
        .get_chain_info(&micro_block1.hash(), false, None,)
        .is_ok());
    assert!(bc_read
        .chain_store
        .get_chain_info(&micro_block2.hash(), false, None,)
        .is_ok());
    assert!(bc_read
        .chain_store
        .get_chain_info(&micro_block3.hash(), false, None,)
        .is_ok());
    assert_eq!(bc_read.block_number(), 1 + Policy::genesis_block_number());

    let mut txs = bc_read.write_transaction();
    // Prune the 3 created MicroBlocks.
    bc_read.chain_store.prune_epoch(1, &mut txs);

    // Check that they no longer exist.
    assert!(bc_read
        .chain_store
        .get_chain_info(&micro_block1.hash(), false, Some(&mut txs),)
        .is_err());
    assert!(bc_read
        .chain_store
        .get_chain_info(&micro_block2.hash(), false, Some(&mut txs),)
        .is_err());
    assert!(bc_read
        .chain_store
        .get_chain_info(&micro_block3.hash(), false, Some(&mut txs),)
        .is_err());
}

#[test]
fn can_detect_invalid_punished_set() {
    let temp_producer = TemporaryBlockProducer::new();
    let config = Default::default();

    // Move blockchain to the end of the batch.
    for _ in 0..Policy::blocks_per_batch() - 1 {
        let block = temp_producer.next_block(vec![], false);
        temp_producer.push(block.clone()).unwrap();
    }

    // Create a macro block with wrong punished set.
    let block = {
        let blockchain = temp_producer.blockchain.read();

        let height = blockchain.block_number() + 1;
        assert!(Policy::is_macro_block_at(height));

        let mut macro_block_proposal =
            next_macro_block_proposal(&temp_producer.producer.signing_key, &blockchain, &config);
        // Put a wrong value into the set.
        macro_block_proposal
            .header
            .next_batch_initial_punished_set
            .insert(2);
        macro_block_proposal.header.body_root = macro_block_proposal.body.as_ref().unwrap().hash();

        let block_hash = macro_block_proposal.hash_blake2s();

        let validators = blockchain
            .get_validators_for_epoch(Policy::epoch_at(blockchain.block_number() + 1), None);
        assert!(validators.is_ok());

        Block::Macro(finalize_macro_block(
            &temp_producer.producer.voting_key,
            ProposalMessage {
                valid_round: None,
                proposal: macro_block_proposal.header,
                round: config.tendermint_round.unwrap_or(0),
            },
            macro_block_proposal.body.unwrap(),
            block_hash,
            &config,
        ))
    };

    assert_eq!(
        temp_producer.push(block),
        Err(PushError::InvalidBlock(BlockError::InvalidValidators))
    );
}

#[test]
fn it_rejects_election_macro_block_proposals_with_wrong_interlink() {
    let temp_producer = TemporaryBlockProducer::new();
    produce_blocks(&temp_producer, Policy::blocks_per_epoch() - 1);

    let blockchain = temp_producer.blockchain.upgradable_read();
    assert!(Policy::is_election_block_at(blockchain.block_number() + 1));

    let mut proposal = temp_producer
        .producer
        .next_macro_block_proposal(
            &blockchain,
            blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
            None,
        )
        .unwrap();
    let mut wrong_interlink = proposal
        .header
        .interlink
        .clone()
        .expect("Election block proposals must contain an interlink");
    wrong_interlink.push(Blake2bHash::default());
    proposal.header.interlink = Some(wrong_interlink);

    assert_eq!(
        blockchain.verify_macro_block_proposal(proposal, 0, None),
        Err(PushError::InvalidBlock(BlockError::InvalidInterlink))
    );
}

#[test]
fn it_rejects_election_macro_block_proposals_with_missing_interlink() {
    let temp_producer = TemporaryBlockProducer::new();
    produce_blocks(&temp_producer, Policy::blocks_per_epoch() - 1);

    let blockchain = temp_producer.blockchain.upgradable_read();
    assert!(Policy::is_election_block_at(blockchain.block_number() + 1));

    let mut proposal = temp_producer
        .producer
        .next_macro_block_proposal(
            &blockchain,
            blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
            None,
        )
        .unwrap();
    proposal.header.interlink = None;

    assert_eq!(
        blockchain.verify_macro_block_proposal(proposal, 0, None),
        Err(PushError::InvalidBlock(BlockError::InvalidInterlink))
    );
}

#[test]
fn it_rejects_non_election_macro_block_proposals_with_superfluous_interlink() {
    let temp_producer = TemporaryBlockProducer::new();
    produce_blocks(&temp_producer, Policy::blocks_per_batch() - 1);

    let blockchain = temp_producer.blockchain.upgradable_read();
    assert!(!Policy::is_election_block_at(blockchain.block_number() + 1));

    let mut proposal = temp_producer
        .producer
        .next_macro_block_proposal(
            &blockchain,
            blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
            None,
        )
        .unwrap();
    proposal.header.interlink = Some(vec![Blake2bHash::default()]);

    assert_eq!(
        blockchain.verify_macro_block_proposal(proposal, 0, None),
        Err(PushError::InvalidBlock(BlockError::InvalidInterlink))
    );
}

/// Seeds a chain at `version`, builds a macro block proposal whose `diff_root` does not match its
/// real state diff, and returns the proposal verification result.
fn verify_macro_proposal_with_wrong_diff_root(version: u16) -> Result<(), PushError> {
    let temp_producer = TemporaryBlockProducer::new_with_protocol_version(version);
    produce_blocks(&temp_producer, Policy::blocks_per_batch() - 1);

    let blockchain = temp_producer.blockchain.upgradable_read();
    let mut proposal = temp_producer
        .producer
        .next_macro_block_proposal(
            &blockchain,
            blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
            None,
        )
        .unwrap();
    proposal.header.diff_root = "invalid diff root".hash();

    blockchain
        .verify_macro_block_proposal(proposal, 0, None)
        .map(|_| ())
}

/// When a validator verifies a macro block proposal, a wrong `diff_root` verifies before
/// `DIFF_ROOT_COMMITMENT` and is rejected from it onward (so honest validators won't sign it).
#[test]
fn it_verifies_the_diff_root_in_a_macro_block_proposal_from_the_commitment_version() {
    assert_all_versions(
        |v| verify_macro_proposal_with_wrong_diff_root(v).is_ok(),
        vec![bp(upgrades::v2::DIFF_ROOT_COMMITMENT, |v| {
            verify_macro_proposal_with_wrong_diff_root(v)
                == Err(PushError::InvalidBlock(BlockError::DiffRootMismatch))
        })],
    );
}
