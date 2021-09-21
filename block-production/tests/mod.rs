use std::sync::Arc;

use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_block::{Block, BlockError, ForkProof};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushError, PushResult};
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_primitives::policy;
use nimiq_test_utils::blockchain::{
    fill_micro_blocks, sign_macro_block, sign_view_change, SECRET_KEY,
};
use nimiq_utils::time::OffsetTime;
use nimiq_vrf::VrfSeed;

#[test]
fn it_can_produce_micro_blocks() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair.clone());

    // #1.0: Empty standard micro block
    let block =
        producer.next_micro_block(blockchain.read().time.now(), 0, None, vec![], vec![0x41]);

    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);

    // Create fork at #1.0
    let fork_proof = {
        let header1 = block.header.clone();
        let justification1 = block.justification.unwrap().signature;
        let mut header2 = header1.clone();
        header2.timestamp += 1;
        let justification2 = keypair.sign(&header2).compress();
        ForkProof {
            header1,
            header2,
            justification1,
            justification2,
        }
    };

    // #2.0: Empty micro block with fork proof
    let block = producer.next_micro_block(
        blockchain.read().time.now() + 1000,
        0,
        None,
        vec![fork_proof],
        vec![0x41],
    );
    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 2);
    assert_eq!(blockchain.read().view_number(), 0);

    // #2.1: Empty view-changed micro block (wrong prev_hash)
    let view_change = sign_view_change(VrfSeed::default(), 3, 1);
    let block = producer.next_micro_block(
        blockchain.read().time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![0x41],
    );

    // the block justification is ok, the view_change justification is not.
    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(block)),
        Err(PushError::InvalidBlock(BlockError::InvalidViewChangeProof))
    );

    // #2.2: Empty view-changed micro block
    let view_change = sign_view_change(blockchain.read().head().seed().clone(), 3, 1);
    let block = producer.next_micro_block(
        blockchain.read().time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![0x41],
    );
    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 3);
    assert_eq!(blockchain.read().next_view_number(), 1);
}

#[test]
fn it_can_produce_macro_blocks() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    fill_micro_blocks(&producer, &blockchain);

    let macro_block = {
        let blockchain = blockchain.read();
        producer.next_macro_block_proposal(
            blockchain.time.now() + blockchain.block_number() as u64 * 1000,
            0u32,
            vec![],
        )
    };

    let block = sign_macro_block(
        &KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap()),
        macro_block.header,
        macro_block.body,
    );
    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Macro(block)),
        Ok(PushResult::Extended)
    );
}

#[test]
fn it_can_produce_election_blocks() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    // push micro and macro blocks until the 3rd epoch is reached
    while policy::epoch_at(blockchain.read().block_number()) < 2 {
        fill_micro_blocks(&producer, &blockchain);

        let macro_block = {
            let blockchain = blockchain.read();
            producer.next_macro_block_proposal(
                blockchain.time.now() + blockchain.block_number() as u64 * 1000,
                0u32,
                vec![0x42],
            )
        };

        let block = sign_macro_block(
            &KeyPair::from(
                SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
            ),
            macro_block.header,
            macro_block.body,
        );

        assert_eq!(
            Blockchain::push(blockchain.upgradable_read(), Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

// TODO Test transactions
