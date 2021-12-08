use parking_lot::RwLock;
use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block::Block;
use nimiq_block_production::{test_utils::TemporaryBlockProducer, BlockProducer};
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_blockchain::{ForkEvent, PushError, PushResult};
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_keys::{KeyPair as SchnorrKeyPair, PrivateKey as SchnorrPrivateKey};
use nimiq_primitives::policy;
use nimiq_test_utils::blockchain::{sign_view_change, SIGNING_KEY, VOTING_KEY};
use nimiq_utils::time::OffsetTime;

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
fn it_can_push_consecutive_view_changes() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let signing_key = SchnorrKeyPair::from(
        SchnorrPrivateKey::deserialize_from_vec(&hex::decode(SIGNING_KEY).unwrap()).unwrap(),
    );
    let voting_key =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(signing_key, voting_key);

    // Produce a simple micro block and push it
    let micro_block = {
        let blockchain = blockchain.read();
        producer.next_micro_block(
            &blockchain,
            blockchain.time.now() + 1_u64 * 1000,
            0,
            None,
            vec![],
            vec![],
            vec![0x42],
        )
    };
    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(micro_block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 1);
    assert_eq!(blockchain.read().view_number(), 0);

    // Produce a micro block with multiple view changes and push it
    let micro_block = {
        let blockchain = blockchain.read();
        let view_change_proof = sign_view_change(blockchain.head().seed().clone(), 2, 5);
        producer.next_micro_block(
            &blockchain,
            blockchain.time.now() + 2_u64 * 1000,
            5,
            Some(view_change_proof),
            vec![],
            vec![],
            vec![0x42],
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
    assert_eq!(blockchain.read().view_number(), 5);

    // Produce one more micro block with a view change
    let micro_block = {
        let blockchain = blockchain.read();

        let view_change_proof = sign_view_change(Block::Micro(micro_block).seed().clone(), 3, 6);
        producer.next_micro_block(
            &blockchain,
            blockchain.time.now() + 3_u64 * 1000,
            6,
            Some(view_change_proof),
            vec![],
            vec![],
            vec![0x42],
        )
    };

    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(micro_block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 3);
    assert_eq!(blockchain.read().view_number(), 6);
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
    // Pushing a block from a previous batch/epoch is atm cought before checking if it's a fork or known block
    assert_eq!(temp_producer1.push(fork), Ok(PushResult::Ignored));
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

    let event1_rc1 = Arc::new(std::sync::RwLock::new(false));
    let event1_rc2 = event1_rc1.clone();

    producer1
        .blockchain
        .write()
        .fork_notifier
        .register(move |e: &ForkEvent| {
            match e {
                ForkEvent::Detected(_) => *event1_rc2.write().unwrap() = true,
            };
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
    assert!(*event1_rc1.read().unwrap());
}
