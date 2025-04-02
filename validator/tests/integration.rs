use std::sync::Arc;

use nimiq_block::Block;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis::NetworkId;
use nimiq_keys::KeyPair;
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{
    produce_macro_blocks_with_txns, signing_key, validator_key, voting_key,
};
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::{key_rng::SecureGenerate, time::OffsetTime};
use parking_lot::RwLock;

#[test(tokio::test)]
async fn validator_update() {
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(
            env.clone(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    // First create an epoch with the original keys
    let producer1 = BlockProducer::new(signing_key(), voting_key());

    // Create new keys and send an update validator tx
    let new_signing_key = KeyPair::generate_default_csprng();
    let new_voting_key = BlsKeyPair::generate_default_csprng();

    let tx = TransactionBuilder::new_update_validator(
        &validator_key(),
        &validator_key(),
        Some(new_signing_key.public),
        Some(&new_voting_key),
        None,
        None,
        Coin::ZERO,
        blockchain.read().block_number(),
        NetworkId::UnitAlbatross,
    );
    let new_micro_block = producer1
        .next_micro_block(
            &blockchain.read(),
            blockchain.read().timestamp() + Policy::BLOCK_SEPARATION_TIME,
            vec![],
            vec![tx],
            vec![0x42],
            None,
        )
        .unwrap();

    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(new_micro_block)),
        Ok(PushResult::Extended)
    );

    // Produce a batch with the new keys
    produce_macro_blocks_with_txns(
        &producer1,
        &blockchain,
        Policy::batches_per_epoch() as usize,
        1,
        2,
    );

    let producer2 = BlockProducer::new(new_signing_key, new_voting_key);
    produce_macro_blocks_with_txns(&producer2, &blockchain, 1, 1, 2);
}
