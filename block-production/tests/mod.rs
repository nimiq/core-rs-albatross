use std::sync::Arc;

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
use nimiq_vrf::VrfSeed;

#[test]
fn it_can_produce_micro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair.clone());

    // #1.0: Empty standard micro block
    let block = producer.next_micro_block(blockchain.time.now(), 0, None, vec![], vec![0x41]);
    assert_eq!(
        blockchain.push(Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.block_number(), 1);

    // Create fork at #1.0
    let fork_proof: ForkProof;
    {
        let header1 = block.header.clone();
        let justification1 = block.justification.unwrap().signature;
        let mut header2 = header1.clone();
        header2.timestamp += 1;
        let justification2 = keypair.sign(&header2).compress();
        fork_proof = ForkProof {
            header1,
            header2,
            justification1,
            justification2,
        };
    }

    // #2.0: Empty micro block with fork proof
    let block = producer.next_micro_block(
        blockchain.time.now() + 1000,
        0,
        None,
        vec![fork_proof],
        vec![0x41],
    );
    assert_eq!(
        blockchain.push(Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.block_number(), 2);
    assert_eq!(blockchain.view_number(), 0);

    // #2.1: Empty view-changed micro block (wrong prev_hash)
    let view_change = sign_view_change(VrfSeed::default(), 3, 1);
    let block = producer.next_micro_block(
        blockchain.time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![0x41],
    );

    // the block justification is ok, the view_change justification is not.
    assert_eq!(
        blockchain.push(Block::Micro(block)),
        Err(PushError::InvalidBlock(BlockError::InvalidViewChangeProof))
    );

    // #2.2: Empty view-changed micro block
    let view_change = sign_view_change(blockchain.head().seed().clone(), 3, 1);
    let block = producer.next_micro_block(
        blockchain.time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![0x41],
    );
    assert_eq!(
        blockchain.push(Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.block_number(), 3);
    assert_eq!(blockchain.next_view_number(), 1);
}

#[test]
fn it_can_produce_macro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    fill_micro_blocks(&producer, &blockchain);

    let macro_block = producer.next_macro_block_proposal(
        blockchain.time.now() + blockchain.block_number() as u64 * 1000,
        0u32,
        vec![],
    );

    let block = sign_macro_block(
        &KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap()),
        macro_block.header,
        macro_block.body,
    );
    assert_eq!(
        blockchain.push(Block::Macro(block)),
        Ok(PushResult::Extended)
    );
}

#[test]
fn it_can_produce_election_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    // push micro and macro blocks until the 3rd epoch is reached
    while policy::epoch_at(blockchain.block_number()) < 2 {
        fill_micro_blocks(&producer, &blockchain);

        let macro_block = producer.next_macro_block_proposal(
            blockchain.time.now() + blockchain.block_number() as u64 * 1000,
            0u32,
            vec![0x42],
        );

        let block = sign_macro_block(
            &KeyPair::from(
                SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
            ),
            macro_block.header,
            macro_block.body,
        );

        assert_eq!(
            blockchain.push(Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

// TODO Test transactions
