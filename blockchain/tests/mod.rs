use std::sync::Arc;
use std::sync::RwLock;

use nimiq_block_production::test_utils::TemporaryBlockProducer;
use nimiq_blockchain::{ForkEvent, PushError, PushResult};
use nimiq_primitives::policy;

#[test]
fn it_can_rebranch_view_changes() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Case 1: easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [1] - [1]
    let block = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(0, vec![]);
    let fork1 = temp_producer2.next_block(1, vec![]);

    let inferior2 = temp_producer1.next_block(0, vec![]);
    let fork2 = temp_producer2.next_block(1, vec![]);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Err(PushError::Orphan));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));

    // Case 2: not obvious rebranch rebranch
    // ... - [1] - [1] - [4]
    //          \- [2] - [2]
    let block = temp_producer1.next_block(1, vec![]);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(1, vec![]);
    let fork1 = temp_producer2.next_block(2, vec![]);

    let inferior2 = temp_producer1.next_block(4, vec![]);
    let fork2 = temp_producer2.next_block(2, vec![]);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Err(PushError::Orphan));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));
}

#[test]
fn it_can_rebranch_forks() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Case 1: easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [0]
    let block = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(block).unwrap();

    let fork1 = temp_producer1.next_block(0, vec![0x48]);
    let fork2 = temp_producer2.next_block(0, vec![]);

    let better = temp_producer1.next_block(0, vec![]);

    // Check that each one accepts other fork.
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Forked));

    // Check that producer 2 rebranches.
    assert_eq!(temp_producer2.push(better), Ok(PushResult::Rebranched));

    // Case 2: more difficult rebranch
    //              a     b     c     d
    // [0] - [0] - [0] - [0] - [0] - [0]
    //          \- [0] - [0] - [1] - [1]
    let block = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(block).unwrap();

    let fork1a = temp_producer1.next_block(0, vec![0x48]);
    let fork2a = temp_producer2.next_block(0, vec![]);

    let fork1b = temp_producer1.next_block(0, vec![]);
    let fork2b = temp_producer2.next_block(0, vec![]);

    let fork1c = temp_producer1.next_block(0, vec![]);
    let fork2c = temp_producer2.next_block(1, vec![]);

    let fork1d = temp_producer1.next_block(0, vec![]);
    let fork2d = temp_producer2.next_block(1, vec![]);

    // Check that each one accepts other fork.
    assert_eq!(temp_producer1.push(fork2a), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1a), Ok(PushResult::Forked));
    assert_eq!(temp_producer1.push(fork2b), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1b), Ok(PushResult::Forked));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork2c), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1c), Ok(PushResult::Ignored));

    assert_eq!(temp_producer1.push(fork2d), Ok(PushResult::Extended));
    assert_eq!(temp_producer2.push(fork1d), Err(PushError::Orphan));
}

#[test]
fn it_cant_rebranch_across_epochs() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // The number in [_] represents the epoch number
    //              a
    // [0] - [0] - [0]
    //          \- [0] - ... - [0] - [1]

    let ancestor = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(ancestor).unwrap();

    // progress the chain across an epoch boundary.
    for _ in 0..policy::EPOCH_LENGTH {
        temp_producer1.next_block(0, vec![]);
    }

    let fork = temp_producer2.next_block(1, vec![]);
    assert_eq!(temp_producer1.push(fork), Err(PushError::InvalidFork));
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
        block = temp_producer1.next_block(0, vec![]);
        temp_producer2.push(block.clone()).unwrap();
        if block.is_macro() {
            break;
        }
    }

    let fork1 = temp_producer1.next_block(0, vec![]);
    let fork2 = temp_producer2.next_block(1, vec![]);

    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Ignored));
}

#[test]
fn create_fork_proof() {
    // Build a fork using two producers.
    let producer1 = TemporaryBlockProducer::new();
    let producer2 = TemporaryBlockProducer::new();

    let event1_rc1 = Arc::new(RwLock::new(false));
    let event1_rc2 = event1_rc1.clone();

    producer1
        .blockchain
        .write()
        .fork_notifier
        .register(move |e: &ForkEvent| match e {
            ForkEvent::Detected(_) => *event1_rc2.write().unwrap() = true,
        });

    // Easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [0]
    let block = producer1.next_block(0, vec![]);
    let _next_block = producer1.next_block(0, vec![0x48]);
    producer2.push(block).unwrap();

    let fork = producer2.next_block(0, vec![]);
    producer1.push(fork).unwrap();

    // Verify that the fork proof was generated
    assert_eq!(*event1_rc1.read().unwrap(), true);
}
