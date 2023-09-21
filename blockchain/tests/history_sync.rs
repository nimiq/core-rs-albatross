use std::sync::Arc;

use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_database::volatile::VolatileDatabase;
use nimiq_genesis::NetworkId;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{
    fill_micro_blocks_with_txns, produce_macro_blocks, produce_macro_blocks_with_txns, signing_key,
    voting_key,
};
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

// Tests if the basic history sync works. It will try to push a succession of election and checkpoint
// blocks. It does test if election blocks can be pushed after checkpoint blocks and vice-versa. It
// does NOT test if macro blocks can be pushed with micro blocks already in the blockchain.
#[test]
fn history_sync_works() {
    let genesis_block_number = Policy::genesis_block_number();
    // The minimum number of macro blocks necessary so that we have two election blocks and a few
    // checkpoint blocks to push.
    let num_macro_blocks = (2 * Policy::batches_per_epoch() + 1) as usize;

    let time = Arc::new(OffsetTime::new());

    // Create a blockchain to produce the macro blocks.
    let env = VolatileDatabase::new(20).unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    // Produce the blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &blockchain, num_macro_blocks);

    let blockchain = blockchain.read();
    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(
            Policy::blocks_per_epoch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let election_txs_1 = blockchain.history_store.get_epoch_transactions(1, None);

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(
            2 * Policy::blocks_per_epoch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let election_txs_2 = blockchain.history_store.get_epoch_transactions(2, None);

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(
            Policy::blocks_per_epoch() + Policy::blocks_per_batch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let mut checkpoint_txs_2_1 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number
            > Policy::blocks_per_epoch() + Policy::blocks_per_batch() + genesis_block_number
        {
            break;
        }

        checkpoint_txs_2_1.push(ext_tx.clone());
    }

    let checkpoint_block_2_3 = blockchain
        .chain_store
        .get_block_at(
            Policy::blocks_per_epoch() + 3 * Policy::blocks_per_batch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let mut checkpoint_txs_2_3 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number
            > Policy::blocks_per_epoch() + 3 * Policy::blocks_per_batch() + genesis_block_number
        {
            break;
        }

        checkpoint_txs_2_3.push(ext_tx.clone());
    }

    let checkpoint_block_3_1 = blockchain
        .chain_store
        .get_block_at(
            2 * Policy::blocks_per_epoch() + Policy::blocks_per_batch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let checkpoint_txs_3_1 = blockchain.history_store.get_epoch_transactions(3, None);

    let time = Arc::new(OffsetTime::new());
    // Create a second blockchain to push these blocks.
    let env2 = VolatileDatabase::new(20).unwrap();

    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(
            env2,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    // Push blocks using history sync.
    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            election_block_1,
            &election_txs_1
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            checkpoint_block_2_1,
            &checkpoint_txs_2_1
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            checkpoint_block_2_3,
            &checkpoint_txs_2_3
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            election_block_2,
            &election_txs_2
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            checkpoint_block_3_1,
            &checkpoint_txs_3_1
        ),
        Ok(PushResult::Extended)
    );
}

// Tests if the history sync works when micro blocks have already been pushed in the blockchain.
// This basically tests if we can go from the history sync to the normal follow mode and back.
#[test]
fn history_sync_works_with_micro_blocks() {
    let genesis_block_number = Policy::genesis_block_number();
    // The minimum number of macro blocks necessary so that we have two election blocks and a few
    // checkpoint blocks to push.
    let num_macro_blocks = (2 * Policy::batches_per_epoch() + 2) as usize;

    let time = Arc::new(OffsetTime::new());

    // Create a blockchain to produce the macro blocks.
    let env = VolatileDatabase::new(20).unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    // Produce the blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(&producer, &blockchain, num_macro_blocks, 5, 0);

    let blockchain = blockchain.read();

    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(
            Policy::blocks_per_epoch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let election_txs_1 = blockchain.history_store.get_epoch_transactions(1, None);

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(
            2 * Policy::blocks_per_epoch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let election_txs_2 = blockchain.history_store.get_epoch_transactions(2, None);

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(
            Policy::blocks_per_epoch() + Policy::blocks_per_batch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let mut checkpoint_txs_2_1 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number
            > Policy::blocks_per_epoch() + Policy::blocks_per_batch() + genesis_block_number
        {
            break;
        }

        checkpoint_txs_2_1.push(ext_tx.clone());
    }

    let checkpoint_block_3_2 = blockchain
        .chain_store
        .get_block_at(
            2 * Policy::blocks_per_epoch() + 2 * Policy::blocks_per_batch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();

    let checkpoint_txs_3_2 = blockchain.history_store.get_epoch_transactions(3, None);

    // Get the micro blocks.
    let mut micro_blocks_2_2 = vec![];

    for i in 1..Policy::blocks_per_batch() {
        micro_blocks_2_2.push(
            blockchain
                .chain_store
                .get_block_at(
                    Policy::blocks_per_epoch()
                        + Policy::blocks_per_batch()
                        + i
                        + genesis_block_number,
                    true,
                    None,
                )
                .unwrap(),
        )
    }

    let mut micro_blocks_3_1 = vec![];

    for i in 1..Policy::blocks_per_batch() {
        micro_blocks_3_1.push(
            blockchain
                .chain_store
                .get_block_at(
                    2 * Policy::blocks_per_epoch() + i + genesis_block_number,
                    true,
                    None,
                )
                .unwrap(),
        )
    }

    let time = Arc::new(OffsetTime::new());
    // Create a second blockchain to push these blocks.
    let env2 = VolatileDatabase::new(20).unwrap();

    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(
            env2,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    // Push blocks using history sync.
    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            election_block_1,
            &election_txs_1
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            checkpoint_block_2_1,
            &checkpoint_txs_2_1
        ),
        Ok(PushResult::Extended)
    );

    // Now go into follow mode.
    for micro in micro_blocks_2_2 {
        assert_eq!(
            Blockchain::push(blockchain2.upgradable_read(), micro),
            Ok(PushResult::Extended)
        );
    }

    // Now go back into history sync.
    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            election_block_2,
            &election_txs_2
        ),
        Ok(PushResult::Extended)
    );

    // Into follow mode one more time.
    for micro in micro_blocks_3_1 {
        assert_eq!(
            Blockchain::push(blockchain2.upgradable_read(), micro),
            Ok(PushResult::Extended)
        );
    }

    // End by going into history sync.
    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            checkpoint_block_3_2,
            &checkpoint_txs_3_2
        ),
        Ok(PushResult::Extended)
    );
}

// Tests if the history sync works when micro blocks have already been pushed in the blockchain,
// but the history given via `push_history_sync` diverges from the adopted one.
#[test]
fn history_sync_works_with_diverging_history() {
    let genesis_block_number = Policy::genesis_block_number();
    // Produce macro blocks to complete one epoch in blockchain1.
    let env = VolatileDatabase::new(20).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    let num_macro_blocks = Policy::batches_per_epoch() as usize;
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(&producer, &blockchain1, num_macro_blocks, 2, 0);
    assert_eq!(
        blockchain1.read().block_number(),
        Policy::blocks_per_epoch() + genesis_block_number
    );

    // Produce some micro blocks (with a different history) in blockchain2.
    let env = VolatileDatabase::new(20).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));
    fill_micro_blocks_with_txns(&producer, &blockchain2, 3, 1);
    assert_eq!(
        blockchain2.read().block_number(),
        Policy::blocks_per_batch() - 1 + genesis_block_number
    );

    // Get the election block and corresponding history tree transactions from blockchain1.
    let blockchain = blockchain1.read();
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(
            Policy::blocks_per_epoch() + genesis_block_number,
            true,
            None,
        )
        .unwrap();
    let election_txs_1 = blockchain.history_store.get_epoch_transactions(1, None);

    // Push the epoch to blockchain2.
    assert_eq!(
        Blockchain::push_history_sync(
            blockchain2.upgradable_read(),
            election_block_1,
            &election_txs_1
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.head(), blockchain2.read().head());
}
