use std::sync::Arc;

use nimiq_block::{Block, BlockError, MacroBlock};
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainError, PushError, PushResult};
use nimiq_database::traits::WriteTransaction;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::{
    networks::NetworkId,
    policy::{upgrades, Policy},
};
use nimiq_tendermint::ProposalMessage;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    test_custom_block::{
        finalize_macro_block, finalize_macro_block_proposal, invalid_voting_key,
        next_macro_block_proposal, replace_voting_keys,
    },
    versions::{assert_all_versions, bp},
};
use nimiq_utils::time::OffsetTime;

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
fn prune_epoch_removes_diffs_of_kept_blocks() {
    // Goal: pruning an epoch removes the accounts diffs of the blocks it keeps
    // (election block, non-prunable checkpoints), but not the blocks themselves.

    let temp_producer = TemporaryBlockProducer::new();
    let blockchain = Arc::clone(&temp_producer.blockchain);

    let election = Policy::election_block_of(1).unwrap();
    let checkpoint = Policy::macro_block_before(election);
    produce_blocks(&temp_producer, election - Policy::genesis_block_number());

    let hash_at = |height| {
        blockchain
            .read()
            .chain_store
            .get_block_at(height, false, None)
            .unwrap()
            .hash()
    };
    let has_diff = |hash: &Blake2bHash| {
        blockchain
            .read()
            .chain_store
            .get_accounts_diff(hash, None)
            .is_ok()
    };
    let micro_hash = hash_at(election - 1);
    let checkpoint_hash = hash_at(checkpoint);
    let election_hash = hash_at(election);
    assert!(has_diff(&micro_hash) && has_diff(&checkpoint_hash) && has_diff(&election_hash));

    // Mark the checkpoint non-prunable, as if its history exceeded `HISTORY_CHUNKS_MAX_SIZE`.
    {
        let bc = blockchain.read();
        let mut chain_info = bc
            .chain_store
            .get_chain_info(&checkpoint_hash, false, None)
            .unwrap();
        chain_info.prunable = false;
        let mut txn = bc.write_transaction();
        bc.chain_store
            .put_chain_info(&mut txn, &checkpoint_hash, &chain_info, false);
        txn.commit();
    }

    // The next election block prunes epoch 1.
    produce_blocks(&temp_producer, Policy::blocks_per_epoch());

    let bc = blockchain.read();
    assert!(bc
        .chain_store
        .get_chain_info(&micro_hash, false, None)
        .is_err());
    // Pruned blocks lose their diffs as well.
    assert!(bc.chain_store.get_accounts_diff(&micro_hash, None).is_err());
    for hash in [&checkpoint_hash, &election_hash] {
        assert!(bc.chain_store.get_chain_info(hash, false, None).is_ok());
        assert_eq!(
            bc.chain_store.get_accounts_diff(hash, None).err(),
            Some(BlockchainError::AccountsDiffNotFound)
        );
    }
    assert_eq!(
        bc.chain_store.get_block_hashes_at(election, None),
        vec![election_hash]
    );
    assert!(bc
        .chain_store
        .get_accounts_diff(&bc.head_hash(), None)
        .is_ok());
}

#[test]
fn load_removes_diffs_left_in_pruned_epochs() {
    // Goal: loading a stored chain removes the accounts diffs that older versions of
    // `prune_epoch` left in every pruned epoch, but not those of the epochs that
    // `max_epochs_stored` still stores.

    let temp_producer = TemporaryBlockProducer::new();
    let blockchain = Arc::clone(&temp_producer.blockchain);

    // In epochs 1 and 2, keep the checkpoint, as if its history exceeded
    // `HISTORY_CHUNKS_MAX_SIZE`, and save the diffs that older versions did not prune.
    let mut leaked = vec![];
    for epoch in 1..=2 {
        let election = Policy::election_block_of(epoch).unwrap();
        let checkpoint = Policy::macro_block_before(election);
        let head = blockchain.read().block_number();
        produce_blocks(&temp_producer, election - head);

        let bc = blockchain.read();
        let checkpoint_hash = bc
            .chain_store
            .get_block_at(checkpoint, false, None)
            .unwrap()
            .hash();
        let mut chain_info = bc
            .chain_store
            .get_chain_info(&checkpoint_hash, false, None)
            .unwrap();
        chain_info.prunable = false;
        let mut txn = bc.write_transaction();
        bc.chain_store
            .put_chain_info(&mut txn, &checkpoint_hash, &chain_info, false);
        txn.commit();

        for height in [checkpoint, election] {
            let hash = bc
                .chain_store
                .get_block_at(height, false, None)
                .unwrap()
                .hash();
            let diff = bc.chain_store.get_accounts_diff(&hash, None).unwrap();
            leaked.push((epoch, hash, diff));
        }
    }

    // The next election block prunes epoch 2, then the head moves into epoch 4.
    produce_blocks(&temp_producer, Policy::blocks_per_epoch() + 1);

    // Puts the diffs back, as older versions left them.
    let put_back = || {
        let bc = blockchain.read();
        let mut txn = bc.write_transaction();
        for (_, hash, diff) in &leaked {
            bc.chain_store.put_accounts_diff(&mut txn, hash, diff);
        }
        txn.commit();
    };
    let (env, genesis) = {
        let bc = blockchain.read();
        let genesis = bc
            .chain_store
            .get_block_at(Policy::genesis_block_number(), false, None)
            .unwrap();
        (bc.state.accounts.env.clone(), genesis)
    };
    let reload = |max_epochs_stored| {
        Blockchain::with_genesis(
            env.clone(),
            BlockchainConfig {
                max_epochs_stored,
                ..Default::default()
            },
            Arc::new(OffsetTime::new()),
            NetworkId::UnitAlbatross,
            genesis.clone(),
            None,
        )
        .unwrap()
    };

    // By default one epoch is stored, so the diffs of epochs 1 and 2 are removed. Zero is
    // raised to that minimum.
    put_back();
    for max_epochs_stored in [Policy::MIN_EPOCHS_STORED, 0] {
        let reloaded = reload(max_epochs_stored);
        for (_, hash, _) in &leaked {
            assert!(reloaded
                .chain_store
                .get_chain_info(hash, false, None)
                .is_ok());
            assert_eq!(
                reloaded.chain_store.get_accounts_diff(hash, None).err(),
                Some(BlockchainError::AccountsDiffNotFound)
            );
        }
        // The epochs still stored keep their diffs, from their first block on.
        for height in [
            Policy::first_block_of(3).unwrap(),
            Policy::election_block_of(3).unwrap(),
            reloaded.block_number(),
        ] {
            let hash = reloaded
                .chain_store
                .get_block_at(height, false, None)
                .unwrap()
                .hash();
            assert!(reloaded.chain_store.get_accounts_diff(&hash, None).is_ok());
        }
    }

    // With two epochs stored, epoch 2 is still stored and keeps its diffs.
    put_back();
    let reloaded = reload(2);
    for (epoch, hash, _) in &leaked {
        assert_eq!(
            reloaded.chain_store.get_accounts_diff(hash, None).is_ok(),
            *epoch == 2
        );
    }
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

/// Builds the next macro block proposal, which must be an election block.
fn next_election_block_proposal(temp_producer: &TemporaryBlockProducer) -> MacroBlock {
    let blockchain = temp_producer.blockchain.read();
    assert!(Policy::is_election_block_at(blockchain.block_number() + 1));
    next_macro_block_proposal(
        &temp_producer.producer.signing_key,
        &blockchain,
        &Default::default(),
    )
}

#[test]
fn it_rejects_justified_election_blocks_with_an_invalid_voting_key() {
    let temp_producer = TemporaryBlockProducer::new();
    produce_blocks(&temp_producer, Policy::blocks_per_epoch() - 1);

    let mut proposal = next_election_block_proposal(&temp_producer);
    proposal.header.validators.as_mut().unwrap().validators[0].voting_key = invalid_voting_key();
    let block_hash = proposal.hash_blake2s();
    let block = finalize_macro_block_proposal(
        &temp_producer.producer.voting_key,
        proposal,
        block_hash,
        &Default::default(),
    );

    // An end-to-end guard: the full node also rejects this with the same error in the post-commit
    // comparison against the staking contract's validators. The light chain has no such
    // comparison, so the light-blockchain tests are what pin the voting key check itself.
    assert_eq!(
        temp_producer.push(block),
        Err(PushError::InvalidBlock(BlockError::InvalidValidators))
    );
}

#[test]
fn it_does_not_decompress_voting_keys_of_unjustified_election_blocks() {
    let temp_producer = TemporaryBlockProducer::new();
    produce_blocks(&temp_producer, Policy::blocks_per_epoch() - 1);

    // Swap in fresh voting keys after computing the signed hash, so the justification doesn't
    // match the block anymore.
    let mut proposal = next_election_block_proposal(&temp_producer);
    let block_hash = proposal.hash_blake2s();
    let voting_keys = replace_voting_keys(proposal.header.validators.as_mut().unwrap());
    let block = finalize_macro_block_proposal(
        &temp_producer.producer.voting_key,
        proposal,
        block_hash,
        &Default::default(),
    );

    assert_eq!(
        temp_producer.push(block),
        Err(PushError::InvalidBlock(BlockError::InvalidJustification))
    );
    assert!(voting_keys.iter().all(|key| !key.has_uncompressed()));
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
