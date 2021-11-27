use std::sync::Arc;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, PushResult};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_primitives::policy::{BATCHES_PER_EPOCH, BATCH_LENGTH, EPOCH_LENGTH};
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

// Tests if the basic history sync works. It will try to push a succession of election and checkpoint
// blocks. It does test if election blocks can be pushed after checkpoint blocks and vice-versa. It
// does NOT test if macro blocks can be pushed with micro blocks already in the blockchain.
#[test]
fn history_sync_works() {
    // The minimum number of macro blocks necessary so that we have two election blocks and a few
    // checkpoint blocks to push.
    let num_macro_blocks = (2 * BATCHES_PER_EPOCH + 1) as usize;

    let time = Arc::new(OffsetTime::new());

    // Create a blockchain to produce the macro blocks.
    let env = VolatileEnvironment::new(10).unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Produce the blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(num_macro_blocks, &producer, &blockchain);

    let blockchain = blockchain.read();
    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_1 = blockchain.history_store.get_epoch_transactions(1, None);

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_2 = blockchain.history_store.get_epoch_transactions(2, None);

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH + BATCH_LENGTH, true, None)
        .unwrap();

    let mut checkpoint_txs_2_1 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number > EPOCH_LENGTH + BATCH_LENGTH {
            break;
        }

        checkpoint_txs_2_1.push(ext_tx.clone());
    }

    let checkpoint_block_2_3 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH + 3 * BATCH_LENGTH, true, None)
        .unwrap();

    let mut checkpoint_txs_2_3 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number > EPOCH_LENGTH + 3 * BATCH_LENGTH {
            break;
        }

        checkpoint_txs_2_3.push(ext_tx.clone());
    }

    let checkpoint_block_3_1 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH + BATCH_LENGTH, true, None)
        .unwrap();

    let checkpoint_txs_3_1 = blockchain.history_store.get_epoch_transactions(3, None);

    let time = Arc::new(OffsetTime::new());
    // Create a second blockchain to push these blocks.
    let env2 = VolatileEnvironment::new(10).unwrap();

    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env2, NetworkId::UnitAlbatross, time).unwrap(),
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
    // The minimum number of macro blocks necessary so that we have two election blocks and a few
    // checkpoint blocks to push.
    let num_macro_blocks = (2 * BATCHES_PER_EPOCH + 2) as usize;

    let time = Arc::new(OffsetTime::new());

    // Create a blockchain to produce the macro blocks.
    let env = VolatileEnvironment::new(10).unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Produce the blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(num_macro_blocks, &producer, &blockchain);

    let blockchain = blockchain.read();

    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_1 = blockchain.history_store.get_epoch_transactions(1, None);

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_2 = blockchain.history_store.get_epoch_transactions(2, None);

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH + BATCH_LENGTH, true, None)
        .unwrap();

    let mut checkpoint_txs_2_1 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number > EPOCH_LENGTH + BATCH_LENGTH {
            break;
        }

        checkpoint_txs_2_1.push(ext_tx.clone());
    }

    let checkpoint_block_3_2 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH + 2 * BATCH_LENGTH, true, None)
        .unwrap();

    let checkpoint_txs_3_2 = blockchain.history_store.get_epoch_transactions(3, None);

    // Get the micro blocks.
    let mut micro_blocks_2_2 = vec![];

    for i in 1..BATCH_LENGTH {
        micro_blocks_2_2.push(
            blockchain
                .chain_store
                .get_block_at(EPOCH_LENGTH + BATCH_LENGTH + i, true, None)
                .unwrap(),
        )
    }

    let mut micro_blocks_3_1 = vec![];

    for i in 1..BATCH_LENGTH {
        micro_blocks_3_1.push(
            blockchain
                .chain_store
                .get_block_at(2 * EPOCH_LENGTH + i, true, None)
                .unwrap(),
        )
    }

    let time = Arc::new(OffsetTime::new());
    // Create a second blockchain to push these blocks.
    let env2 = VolatileEnvironment::new(10).unwrap();

    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env2, NetworkId::UnitAlbatross, time).unwrap(),
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
