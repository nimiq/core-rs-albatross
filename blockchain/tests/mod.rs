use parking_lot::RwLock;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use nimiq_block::Block;
use nimiq_block_production::{test_utils::TemporaryBlockProducer, BlockProducer};
use nimiq_blockchain::PushResult;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{signing_key, voting_key};
use nimiq_utils::time::OffsetTime;

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
fn it_can_push_consecutive_view_changes() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let producer = BlockProducer::new(signing_key(), voting_key());

    // Produce a simple micro block and push it
    let micro_block = {
        let blockchain = blockchain.read();
        producer.next_micro_block(
            &blockchain,
            blockchain.time.now() + 1_u64 * 1000,
            vec![],
            vec![],
            vec![0x42],
            None,
        )
    };
    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(micro_block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 1);

    // Produce a micro block with multiple view changes and push it
    let micro_block = {
        let blockchain = blockchain.read();
        producer.next_micro_block(
            &blockchain,
            blockchain.time.now() + 2_u64 * 1000,
            vec![],
            vec![],
            vec![0x42],
            None,
        )
    };
    assert_eq!(
        Blockchain::push(
            blockchain.upgradable_read(),
            Block::Micro(micro_block.clone())
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    // Produce one more micro block with a view change
    let micro_block = {
        let blockchain = blockchain.read();

        producer.next_micro_block(
            &blockchain,
            blockchain.time.now() + 3_u64 * 1000,
            vec![],
            vec![],
            vec![0x42],
            None,
        )
    };

    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(micro_block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 3);
}

#[test]
fn micro_block_works_after_macro_block() {
    let temp_producer = TemporaryBlockProducer::new();

    // apply an entire batch including macro block on view_number/round_number zero
    for _ in 0..Policy::blocks_per_batch() {
        temp_producer.next_block(vec![], false);
    }
    // make sure we are at the beginning of the batch and all block were applied
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch()
    );

    // Test if a micro block can be rebranched immediately after
    // a round_number 0 macro block

    // create a couple of skip blocks
    let block = temp_producer.next_block_no_push(vec![], true);
    let rebranch = temp_producer.next_block_no_push(vec![], true);
    // push first skip block
    temp_producer.push(block).unwrap();
    // make sure this was an extend
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + 1
    );
    // and rebranch it to block the chain with only one skip block
    temp_producer.push(rebranch).unwrap();
    // make sure this was a rebranch
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + 1
    );

    // apply the rest of the batch including macro block on view_number/round_number one
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }
    // make sure we are at the beginning of the batch
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() * 2
    );

    // Test if a micro block can be rebranched immediately after
    // a round_number non 0 macro block

    // create blocks for a chain with accumulated skip blocks after the batch of 0, 1 and 2
    let block = temp_producer.next_block_no_push(vec![], true);
    let rebranch1 = temp_producer.next_block_no_push(vec![], true);
    // let rebranch2 = temp_producer.next_block_no_push(2, vec![]);
    // apply them each rebranching the previous one
    temp_producer.push(block).unwrap();
    temp_producer.push(rebranch1).unwrap();
    // temp_producer.push(rebranch2).unwrap();

    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() * 2 + 1
    );
}

#[test]
fn it_can_rebranch_forks() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

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
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // The numbers in [X/Y] represent block_number (X) and view_number (Y):
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
    let producer1 = TemporaryBlockProducer::new();
    let producer2 = TemporaryBlockProducer::new();

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

#[test(tokio::test)]
async fn create_fork_proof() {
    // Build a fork using two producers.
    let producer1 = TemporaryBlockProducer::new();
    let producer2 = TemporaryBlockProducer::new();

    let mut fork_rx = BroadcastStream::new(producer1.blockchain.read().fork_notifier.subscribe());

    // Easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [0]
    let block = producer1.next_block(vec![], false);
    let _next_block = producer1.next_block(vec![0x48], false);
    producer2.push(block).unwrap();

    let fork = producer2.next_block(vec![], false);
    producer1.push(fork).unwrap();

    // Verify that the fork proof was generated
    assert!(fork_rx.next().await.is_some());
}
