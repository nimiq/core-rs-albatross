use std::sync::Arc;

use nimiq_block::Block;
use nimiq_blockchain::{interface::HistoryInterface, BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis::NetworkId;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{signing_key, voting_key},
};
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

/// A node that follows the chain of another `TemporaryBlockProducer`.
fn node(config: BlockchainConfig) -> TemporaryBlockProducer {
    TemporaryBlockProducer {
        blockchain: Arc::new(RwLock::new(
            Blockchain::new(
                MdbxDatabase::new_volatile(Default::default()).unwrap(),
                config,
                NetworkId::UnitAlbatross,
                Arc::new(OffsetTime::new()),
            )
            .unwrap(),
        )),
        producer: BlockProducer::new(signing_key(), voting_key()),
    }
}

/// The producer of the chain that the nodes follow. It keeps every block, so that
/// `blocks_in_epoch` can look up the blocks that a node pruned.
fn producer() -> TemporaryBlockProducer {
    let temporary_producer = TemporaryBlockProducer::new();
    temporary_producer
        .blockchain
        .write()
        .config
        .max_epochs_stored = u32::MAX;
    temporary_producer
}

/// Produces the blocks up to `block_number` and returns the last one.
fn produce_up_to(producer: &TemporaryBlockProducer, block_number: u32) -> Block {
    let mut block = None;
    while producer.blockchain.read().block_number() < block_number {
        block = Some(producer.next_block(vec![], false));
    }
    block.unwrap()
}

/// Produces the blocks up to `block_number` and pushes each of them to the node.
fn extend_up_to(
    producer: &TemporaryBlockProducer,
    node: &TemporaryBlockProducer,
    block_number: u32,
) {
    while producer.blockchain.read().block_number() < block_number {
        let block = producer.next_block(vec![], false);
        assert_eq!(node.push(block), Ok(PushResult::Extended));
    }
}

/// Produces the blocks up to the macro block `block_number` and pushes only that one to the node,
/// through history sync.
fn history_sync_up_to(
    producer: &TemporaryBlockProducer,
    node: &TemporaryBlockProducer,
    block_number: u32,
) {
    let block = produce_up_to(producer, block_number);
    let history = producer
        .blockchain
        .read()
        .history_store
        .get_epoch_transactions(block.epoch_number(), None);
    assert_eq!(
        Blockchain::push_history_sync(node.blockchain.upgradable_read(), block, &history),
        Ok(PushResult::Extended)
    );
}

/// The node takes a skip block that the producer's chain lacks. That chain stays inferior until
/// its next macro block arrives.
fn rebranch_to_next_macro_block(producer: &TemporaryBlockProducer, node: &TemporaryBlockProducer) {
    node.next_block(vec![], true);
    loop {
        let block = producer.next_block(vec![], false);
        if block.is_macro() {
            assert_eq!(node.push(block), Ok(PushResult::Rebranched));
            break;
        }
        assert_eq!(node.push(block), Ok(PushResult::Ignored));
    }
}

/// The number of blocks of `epoch` in the producer's chain, the election block included, that the
/// node stores. It looks them up by hash, so a block that lost its height index entry still
/// counts.
fn blocks_in_epoch(
    producer: &TemporaryBlockProducer,
    node: &TemporaryBlockProducer,
    epoch: u32,
) -> u32 {
    let producer = producer.blockchain.read();
    let node = node.blockchain.read();
    (Policy::first_block_of(epoch).unwrap()..=Policy::election_block_of(epoch).unwrap())
        .filter(|&height| {
            let hash = producer.get_block_at(height, false, None).unwrap().hash();
            node.chain_store.get_chain_info(&hash, false, None).is_ok()
        })
        .count() as u32
}

fn history_len(node: &TemporaryBlockProducer, epoch: u32) -> usize {
    let blockchain = node.blockchain.read();
    blockchain
        .history_store
        .get_epoch_transactions(epoch, None)
        .len()
}

// Tests that an election block that extends the chain prunes the epoch that leaves the stored
// range. Checkpoint blocks prune nothing. Without `keep_history` (full node), the history of the
// pruned epoch goes as well.
#[test]
fn extending_prunes_the_epoch_that_leaves_the_stored_range() {
    for keep_history in [true, false] {
        let producer = producer();
        let node = node(BlockchainConfig {
            keep_history,
            ..Default::default()
        });
        let election2 = Policy::election_block_of(2).unwrap();

        extend_up_to(&producer, &node, election2 - 1);
        assert_eq!(
            blocks_in_epoch(&producer, &node, 1),
            Policy::blocks_per_epoch()
        );

        extend_up_to(&producer, &node, election2);
        assert_eq!(blocks_in_epoch(&producer, &node, 1), 1);
        assert_eq!(history_len(&node, 1) > 0, keep_history);
    }
}

// Tests that a node stores at least `Policy::MIN_EPOCHS_STORED` epochs, also when
// `max_epochs_stored` is lower. With no epoch stored, the election block of epoch 1 would prune
// its own epoch.
#[test]
fn at_least_one_epoch_is_stored() {
    let producer = producer();
    let node = node(BlockchainConfig {
        max_epochs_stored: 0,
        ..Default::default()
    });

    extend_up_to(&producer, &node, Policy::election_block_of(1).unwrap());
    assert_eq!(
        blocks_in_epoch(&producer, &node, 1),
        Policy::blocks_per_epoch()
    );

    extend_up_to(&producer, &node, Policy::election_block_of(2).unwrap());
    assert_eq!(blocks_in_epoch(&producer, &node, 1), 1);
    assert_eq!(
        blocks_in_epoch(&producer, &node, 2),
        Policy::blocks_per_epoch()
    );
}

// Tests that an election block pushed through macro sync prunes the epoch that leaves the stored
// range, with its history. Checkpoint blocks prune nothing.
#[test]
fn macro_sync_prunes_the_epoch_that_leaves_the_stored_range() {
    let producer = producer();
    let node = node(BlockchainConfig {
        keep_history: false,
        ..Default::default()
    });
    let push_macro_up_to = |block_number| {
        let block = produce_up_to(&producer, block_number);
        assert_eq!(
            Blockchain::push_macro(node.blockchain.upgradable_read(), block),
            Ok(PushResult::Extended)
        );
    };
    let election1 = Policy::election_block_of(1).unwrap();
    extend_up_to(&producer, &node, election1);

    push_macro_up_to(Policy::macro_block_after(election1));
    assert_eq!(
        blocks_in_epoch(&producer, &node, 1),
        Policy::blocks_per_epoch()
    );
    assert!(history_len(&node, 1) > 0);

    push_macro_up_to(Policy::election_block_after(election1));
    assert_eq!(blocks_in_epoch(&producer, &node, 1), 1);
    assert_eq!(history_len(&node, 1), 0);
}

// Tests that history sync prunes nothing. The epoch that is due at an election block that arrives
// through history sync goes at the next live election block, with the catch-up. The blocks that
// history sync stored stay.
#[test]
fn history_sync_leaves_pruning_to_the_next_live_election_block() {
    let producer = producer();
    let node = node(BlockchainConfig::default());
    let election1 = Policy::election_block_of(1).unwrap();
    let election2 = Policy::election_block_of(2).unwrap();
    extend_up_to(&producer, &node, election1);

    // Epoch 2 arrives through history sync: a checkpoint block, then the election block at which
    // epoch 1 is due. Neither prunes.
    history_sync_up_to(&producer, &node, Policy::macro_block_after(election1));
    history_sync_up_to(&producer, &node, election2);
    assert_eq!(
        blocks_in_epoch(&producer, &node, 1),
        Policy::blocks_per_epoch()
    );

    // The node follows the chain again. The next election block prunes epoch 2, which is due, and
    // catches up on epoch 1. Epoch 2 keeps the two blocks that history sync stored.
    extend_up_to(&producer, &node, Policy::election_block_of(3).unwrap());
    assert_eq!(blocks_in_epoch(&producer, &node, 1), 1);
    assert_eq!(blocks_in_epoch(&producer, &node, 2), 2);
    assert_eq!(
        blocks_in_epoch(&producer, &node, 3),
        Policy::blocks_per_epoch()
    );
}

// Tests that a rebranch onto an election block prunes the epoch that leaves the stored range,
// like an election block that extends the chain, and on a full node its history. A rebranch onto
// a checkpoint block prunes nothing.
#[test]
fn rebranch_to_election_block_prunes_the_epoch_that_leaves_the_stored_range() {
    let producer = producer();
    let node = node(BlockchainConfig {
        keep_history: false,
        ..Default::default()
    });
    let election2 = Policy::election_block_of(2).unwrap();
    let last_checkpoint = Policy::macro_block_before(election2);
    extend_up_to(
        &producer,
        &node,
        Policy::macro_block_before(last_checkpoint),
    );

    // A rebranch onto the last checkpoint block of epoch 2 prunes nothing. The last one, because a
    // macro block that extends the chain after a rebranch drops the blocks that the rebranch
    // adopted from the height index, and pruning no longer finds them (a separate bug).
    rebranch_to_next_macro_block(&producer, &node);
    assert_eq!(node.blockchain.read().block_number(), last_checkpoint);
    assert_eq!(
        blocks_in_epoch(&producer, &node, 1),
        Policy::blocks_per_epoch()
    );

    // A rebranch onto the election block of epoch 2 prunes epoch 1 down to the election block,
    // with its history.
    rebranch_to_next_macro_block(&producer, &node);
    assert_eq!(node.blockchain.read().block_number(), election2);
    assert_eq!(blocks_in_epoch(&producer, &node, 1), 1);
    assert_eq!(history_len(&node, 1), 0);

    // Epoch 2 is still stored, with its history.
    assert_eq!(
        blocks_in_epoch(&producer, &node, 2),
        Policy::blocks_per_epoch()
    );
    assert!(history_len(&node, 2) > 0);

    // The node keeps following the chain. The next election block extends it and prunes epoch 2.
    extend_up_to(&producer, &node, Policy::election_block_of(3).unwrap());
    assert_eq!(blocks_in_epoch(&producer, &node, 2), 1);
    assert_eq!(history_len(&node, 2), 0);
}

// Tests that an election block also prunes the oldest epoch that was left behind, one per
// election block. Epochs are left behind if `max_epochs_stored` is lowered, if the election block
// at which they were due arrived through history sync, or if older versions skipped them after a
// rebranch.
#[test]
fn election_block_catches_up_on_epochs_left_behind() {
    let producer = producer();
    let node = node(BlockchainConfig {
        max_epochs_stored: 10,
        ..Default::default()
    });
    let election = |epoch| Policy::election_block_of(epoch).unwrap();
    let blocks_per_epoch = Policy::blocks_per_epoch();

    // Epoch 1 arrives through history sync. Its checkpoint and election block are not prunable.
    history_sync_up_to(&producer, &node, Policy::macro_block_before(election(1)));
    history_sync_up_to(&producer, &node, election(1));
    // Epochs 2 to 4 are applied live and stay stored.
    extend_up_to(&producer, &node, election(4));
    let stored = |epochs: std::ops::RangeInclusive<u32>| -> Vec<u32> {
        epochs
            .map(|epoch| blocks_in_epoch(&producer, &node, epoch))
            .collect()
    };
    assert_eq!(
        stored(1..=4),
        [2, blocks_per_epoch, blocks_per_epoch, blocks_per_epoch]
    );

    // From here on the node stores one epoch, which leaves epochs 2 and 3 behind.
    node.blockchain.write().config.max_epochs_stored = 1;

    // The election block of epoch 5 prunes epoch 4, which is due, and epoch 2.
    extend_up_to(&producer, &node, election(5));
    assert_eq!(stored(1..=5), [2, 1, blocks_per_epoch, 1, blocks_per_epoch]);

    // The election block of epoch 6 prunes epoch 5 and epoch 3.
    extend_up_to(&producer, &node, election(6));
    assert_eq!(stored(1..=6), [2, 1, 1, 1, 1, blocks_per_epoch]);

    // The election blocks are all kept, and so is the history.
    let blockchain = node.blockchain.read();
    for epoch in 1..=6 {
        assert!(blockchain
            .get_block_at(election(epoch), false, None)
            .is_ok());
        assert!(!blockchain
            .history_store
            .get_epoch_transactions(epoch, None)
            .is_empty());
    }
}

// Tests that the history of an epoch that was left behind goes with it when the node does not
// keep the history (full node).
#[test]
fn full_node_catches_up_on_the_history_of_epochs_left_behind() {
    let producer = producer();
    let node = node(BlockchainConfig {
        keep_history: false,
        max_epochs_stored: 10,
        ..Default::default()
    });
    let election = |epoch| Policy::election_block_of(epoch).unwrap();

    extend_up_to(&producer, &node, election(3));
    node.blockchain.write().config.max_epochs_stored = 1;

    // The election block of epoch 4 prunes epoch 3, which is due, and epoch 1.
    extend_up_to(&producer, &node, election(4));
    assert_eq!(blocks_in_epoch(&producer, &node, 1), 1);
    assert_eq!(
        blocks_in_epoch(&producer, &node, 2),
        Policy::blocks_per_epoch()
    );
    assert_eq!(blocks_in_epoch(&producer, &node, 3), 1);
    assert_eq!(history_len(&node, 1), 0);
    assert!(history_len(&node, 2) > 0);
    assert_eq!(history_len(&node, 3), 0);

    // The election block of epoch 5 prunes epoch 4 and epoch 2.
    extend_up_to(&producer, &node, election(5));
    assert_eq!(blocks_in_epoch(&producer, &node, 2), 1);
    assert_eq!(history_len(&node, 2), 0);
}

// Tests that a checkpoint block is enough for an epoch to count as left behind. Macro sync leaves
// such an epoch: one prunable checkpoint block and no micro blocks.
#[test]
fn election_block_catches_up_on_a_checkpoint_block_left_behind() {
    let producer = producer();
    let node = node(BlockchainConfig {
        keep_history: false,
        max_epochs_stored: 10,
        ..Default::default()
    });
    let push_macro_up_to = |block_number| {
        let block = produce_up_to(&producer, block_number);
        assert_eq!(
            Blockchain::push_macro(node.blockchain.upgradable_read(), block),
            Ok(PushResult::Extended)
        );
    };
    let election = |epoch| Policy::election_block_of(epoch).unwrap();

    push_macro_up_to(Policy::macro_block_before(election(1)));
    push_macro_up_to(election(1));
    push_macro_up_to(election(2));
    assert_eq!(blocks_in_epoch(&producer, &node, 1), 2);

    // Epoch 2 is due at the election block of epoch 3 and holds nothing to prune.
    node.blockchain.write().config.max_epochs_stored = 1;
    push_macro_up_to(election(3));
    assert_eq!(blocks_in_epoch(&producer, &node, 1), 1);
}
