use nimiq_primitives::coin::Coin;
use parking_lot::RwLock;
use std::convert::TryInto;
use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block::{Block, BlockError, ForkProof};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushError, PushResult};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_bls::SecretKey;

use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::policy;
use nimiq_test_utils::blockchain::{
    fill_micro_blocks, sign_macro_block, sign_view_change, SECRET_KEY,
};
use nimiq_utils::time::OffsetTime;
use nimiq_vrf::VrfSeed;

use nimiq_transaction_builder::TransactionBuilder;

const ADDRESS: &str = "NQ20TSB0DFSMUH9C15GQGAGJTTE4D3MA859E";
pub const WARM_SECRET_KEY: &str =
    "041580cc67e66e9e08b68fd9e4c9deb68737168fbe7488de2638c2e906c2f5ad";

pub const ACCOUNT_SECRET_KEY: &str =
    "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";

const STAKER_ADDRESS: &str = "NQ20TSB0DFSMUH9C15GQGAGJTTE4D3MA859E";

#[test]
fn it_can_produce_micro_blocks() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let keypair = BlsKeyPair::from(
        SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
    );
    let producer = BlockProducer::new(keypair.clone());

    let bc = blockchain.upgradable_read();
    // #1.0: Empty standard micro block
    let block = producer.next_micro_block(&bc, bc.time.now(), 0, None, vec![], vec![], vec![0x41]);

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
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

    let bc = blockchain.upgradable_read();
    // #2.0: Empty micro block with fork proof
    let block = producer.next_micro_block(
        &bc,
        bc.time.now() + 1000,
        0,
        None,
        vec![fork_proof],
        vec![],
        vec![0x41],
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);
    assert_eq!(blockchain.read().view_number(), 0);

    // #2.1: Empty view-changed micro block (wrong prev_hash)
    let view_change = sign_view_change(VrfSeed::default(), 3, 1);
    let bc = blockchain.upgradable_read();
    let block = producer.next_micro_block(
        &bc,
        bc.time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![],
        vec![0x41],
    );

    // the block justification is ok, the view_change justification is not.
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Err(PushError::InvalidBlock(BlockError::InvalidViewChangeProof))
    );

    // #2.2: Empty view-changed micro block
    let view_change = sign_view_change(blockchain.read().head().seed().clone(), 3, 1);
    let bc = blockchain.upgradable_read();
    let block = producer.next_micro_block(
        &bc,
        bc.time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![],
        vec![0x41],
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
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

    let keypair = BlsKeyPair::from(
        SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
    );
    let producer = BlockProducer::new(keypair);

    fill_micro_blocks(&producer, &blockchain);

    let bc = blockchain.upgradable_read();
    let macro_block = {
        producer.next_macro_block_proposal(
            &bc,
            bc.time.now() + bc.block_number() as u64 * 1000,
            0u32,
            vec![],
        )
    };

    let block = sign_macro_block(
        &BlsKeyPair::from(
            SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
        ),
        macro_block.header,
        macro_block.body,
    );
    assert_eq!(
        Blockchain::push(bc, Block::Macro(block)),
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

    let keypair = BlsKeyPair::from(
        SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
    );

    let producer = BlockProducer::new(keypair);

    // push micro and macro blocks until the 3rd epoch is reached
    while policy::epoch_at(blockchain.read().block_number()) < 2 {
        fill_micro_blocks(&producer, &blockchain);

        let bc = blockchain.upgradable_read();
        let macro_block = {
            producer.next_macro_block_proposal(
                &bc,
                bc.time.now() + bc.block_number() as u64 * 1000,
                0u32,
                vec![0x42],
            )
        };

        let block = sign_macro_block(
            &BlsKeyPair::from(
                SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
            ),
            macro_block.header,
            macro_block.body,
        );

        assert_eq!(
            Blockchain::push(bc, Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

#[test]
fn it_can_revert_unpark_transactions() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let keypair = BlsKeyPair::from(
        SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
    );
    let producer = BlockProducer::new(keypair.clone());

    // #1.0: Empty view-changed micro block
    let view_change = sign_view_change(blockchain.read().head().seed().clone(), 1, 1);
    let bc = blockchain.upgradable_read();

    let block = producer.next_micro_block(
        &bc,
        bc.time.now(),
        1,
        Some(view_change),
        vec![],
        vec![],
        vec![0x41],
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);
    assert_eq!(blockchain.read().next_view_number(), 1);

    let bc = blockchain.upgradable_read();

    // One empty block
    let block = producer.next_micro_block(
        &bc,
        bc.time.now() + 2000,
        1,
        None,
        vec![],
        vec![],
        vec![0x41],
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);
    assert_eq!(blockchain.read().next_view_number(), 1);

    // One block with stacking transactions

    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(WARM_SECRET_KEY);
    let address = Address::from_any_str(ADDRESS).unwrap();

    let tx = TransactionBuilder::new_unpark_validator(
        &key_pair,
        address.clone(),
        &key_pair,
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.time.now() + 2000,
        1,
        None,
        vec![],
        transactions,
        vec![0x41],
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 3);
    assert_eq!(blockchain.read().next_view_number(), 1);

    let bc = blockchain.upgradable_read();

    let mut txn = bc.write_transaction();

    let result = bc.revert_blocks(3, &mut txn);

    assert_eq!(result, Ok(()));
}

#[test]
fn it_can_revert_create_stacker_transaction() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let keypair = BlsKeyPair::from(
        SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
    );
    let producer = BlockProducer::new(keypair.clone());

    // #1.0: Empty view-changed micro block
    let view_change = sign_view_change(blockchain.read().head().seed().clone(), 1, 1);
    let bc = blockchain.upgradable_read();

    let block = producer.next_micro_block(
        &bc,
        bc.time.now(),
        1,
        Some(view_change),
        vec![],
        vec![],
        vec![0x41],
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 1);
    assert_eq!(blockchain.read().next_view_number(), 1);

    let bc = blockchain.upgradable_read();

    // One empty block
    let block = producer.next_micro_block(
        &bc,
        bc.time.now() + 2000,
        1,
        None,
        vec![],
        vec![],
        vec![0x41],
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);
    assert_eq!(blockchain.read().next_view_number(), 1);

    // One block with stacking transactions

    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let tx = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair,
        Some(address.clone()),
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.time.now() + 2000,
        1,
        None,
        vec![],
        transactions,
        vec![0x41],
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 3);
    assert_eq!(blockchain.read().next_view_number(), 1);

    let bc = blockchain.upgradable_read();

    let mut txn = bc.write_transaction();
    let result = bc.revert_blocks(3, &mut txn);

    assert_eq!(result, Ok(()));
}

fn ed25519_key_pair(secret_key: &str) -> KeyPair {
    let priv_key: PrivateKey =
        Deserialize::deserialize(&mut &hex::decode(secret_key).unwrap()[..]).unwrap();
    priv_key.into()
}
