use std::sync::Arc;

use nimiq_block::{Block, BlockError};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_hash::Hash;
use nimiq_primitives::policy::Policy;
use nimiq_tendermint::ProposalMessage;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    test_custom_block::{finalize_macro_block, next_macro_block_proposal},
};

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
        producer.next_micro_block(
            &bc_read,
            &bc_read.time.now() + 1_u64 * 1000,
            vec![],
            vec![],
            vec![0x42],
            None,
        )
    };
    let micro_block2 = {
        let bc_read = blockchain.read();
        producer.next_micro_block(
            &bc_read,
            bc_read.time.now() + 1_u64 * 100,
            vec![],
            vec![],
            vec![0x32],
            None,
        )
    };
    let micro_block3 = {
        let bc_read = blockchain.read();
        producer.next_micro_block(
            &bc_read,
            bc_read.time.now() + 1_u64 * 10000,
            vec![],
            vec![],
            vec![0x82],
            None,
        )
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
            .body
            .as_mut()
            .unwrap()
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
