use std::sync::Arc;

use parking_lot::RwLock;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushResult, CHUNK_SIZE};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_primitives::policy::{BATCHES_PER_EPOCH, BLOCKS_PER_BATCH, BLOCKS_PER_EPOCH};
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{
    fill_micro_blocks_with_txns, produce_macro_blocks, produce_macro_blocks_with_txns, signing_key,
    voting_key,
};
use nimiq_utils::time::OffsetTime;

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
    produce_macro_blocks(&producer, &blockchain, num_macro_blocks);

    let blockchain = blockchain.read();
    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(BLOCKS_PER_EPOCH, true, None)
        .unwrap();

    let election_txs_1 = blockchain
        .history_store
        .prove_chunk(
            election_block_1.epoch_number(),
            election_block_1.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(2 * BLOCKS_PER_EPOCH, true, None)
        .unwrap();

    let election_txs_2 = blockchain
        .history_store
        .prove_chunk(
            election_block_2.epoch_number(),
            election_block_2.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(BLOCKS_PER_EPOCH + BLOCKS_PER_BATCH, true, None)
        .unwrap();

    let checkpoint_txs_2_1 = blockchain
        .history_store
        .prove_chunk(
            checkpoint_block_2_1.epoch_number(),
            checkpoint_block_2_1.block_number(),
            CHUNK_SIZE as usize,
            0,
            None,
        )
        .unwrap();

    let checkpoint_block_2_3 = blockchain
        .chain_store
        .get_block_at(BLOCKS_PER_EPOCH + 3 * BLOCKS_PER_BATCH, true, None)
        .unwrap();

    let checkpoint_txs_2_3 = blockchain
        .history_store
        .prove_chunk(
            checkpoint_block_2_3.epoch_number(),
            checkpoint_block_2_3.block_number(),
            CHUNK_SIZE as usize,
            0,
            None,
        )
        .unwrap();

    let checkpoint_block_3_1 = blockchain
        .chain_store
        .get_block_at(2 * BLOCKS_PER_EPOCH + BLOCKS_PER_BATCH, true, None)
        .unwrap();

    let checkpoint_txs_3_1 = blockchain
        .history_store
        .prove_chunk(
            checkpoint_block_3_1.epoch_number(),
            checkpoint_block_3_1.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    let time = Arc::new(OffsetTime::new());
    // Create a second blockchain to push these blocks.
    let env2 = VolatileEnvironment::new(10).unwrap();

    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env2, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Push election_block_1 associated transactions

    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        election_block_1.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(blockchain2.upgradable_read(), election_txs_1, 0, CHUNK_SIZE),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );

    // Push checkpoint_block_2_1 associated transactions

    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        checkpoint_block_2_1.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(
            blockchain2.upgradable_read(),
            checkpoint_txs_2_1,
            0,
            CHUNK_SIZE
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );

    // Push checkpoint_block_2_3 associated transactions

    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        checkpoint_block_2_3.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(
            blockchain2.upgradable_read(),
            checkpoint_txs_2_3,
            0,
            CHUNK_SIZE
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );

    // Push election_block_2 associated transactions

    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        election_block_2.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(blockchain2.upgradable_read(), election_txs_2, 0, CHUNK_SIZE),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );

    // Push checkpoint_block_3_1 associated transactions

    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        checkpoint_block_3_1.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(
            blockchain2.upgradable_read(),
            checkpoint_txs_3_1,
            0,
            CHUNK_SIZE
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
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
    produce_macro_blocks_with_txns(&producer, &blockchain, num_macro_blocks, 5, 0);

    let blockchain = blockchain.read();

    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(BLOCKS_PER_EPOCH, true, None)
        .unwrap();

    let election_txs_1 = blockchain
        .history_store
        .prove_chunk(
            election_block_1.epoch_number(),
            election_block_1.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(2 * BLOCKS_PER_EPOCH, true, None)
        .unwrap();

    let election_txs_2 = blockchain
        .history_store
        .prove_chunk(
            election_block_2.epoch_number(),
            election_block_2.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(BLOCKS_PER_EPOCH + BLOCKS_PER_BATCH, true, None)
        .unwrap();

    let checkpoint_txs_2_1 = blockchain
        .history_store
        .prove_chunk(
            checkpoint_block_2_1.epoch_number(),
            checkpoint_block_2_1.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    let checkpoint_block_3_2 = blockchain
        .chain_store
        .get_block_at(2 * BLOCKS_PER_EPOCH + 2 * BLOCKS_PER_BATCH, true, None)
        .unwrap();

    let checkpoint_txs_3_2 = blockchain
        .history_store
        .prove_chunk(
            checkpoint_block_3_2.epoch_number(),
            checkpoint_block_3_2.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    // Get the micro blocks.
    let mut micro_blocks_2_2 = vec![];

    for i in 1..BLOCKS_PER_BATCH {
        micro_blocks_2_2.push(
            blockchain
                .chain_store
                .get_block_at(BLOCKS_PER_EPOCH + BLOCKS_PER_BATCH + i, true, None)
                .unwrap(),
        )
    }

    let mut micro_blocks_3_1 = vec![];

    for i in 1..BLOCKS_PER_BATCH {
        micro_blocks_3_1.push(
            blockchain
                .chain_store
                .get_block_at(2 * BLOCKS_PER_EPOCH + i, true, None)
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
    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        election_block_1.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(blockchain2.upgradable_read(), election_txs_1, 0, CHUNK_SIZE),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );

    // Push blocks using history sync.
    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        checkpoint_block_2_1.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(
            blockchain2.upgradable_read(),
            checkpoint_txs_2_1,
            0,
            CHUNK_SIZE
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
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
    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        election_block_2.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(blockchain2.upgradable_read(), election_txs_2, 0, CHUNK_SIZE),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
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
    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        checkpoint_block_3_2.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(
            blockchain2.upgradable_read(),
            checkpoint_txs_3_2,
            0,
            CHUNK_SIZE
        ),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );
}

// Tests if the history sync works when micro blocks have already been pushed in the blockchain,
// but the history given via `push_history_sync` diverges from the adopted one.
#[test]
fn history_sync_works_with_diverging_history() {
    // Produce macro blocks to complete one epoch in blockchain1.
    let env = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    let num_macro_blocks = BATCHES_PER_EPOCH as usize;
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(&producer, &blockchain1, num_macro_blocks, 2, 0);
    assert_eq!(blockchain1.read().block_number(), BLOCKS_PER_EPOCH);

    // Produce some micro blocks (with a different history) in blockchain2.
    let env = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));
    fill_micro_blocks_with_txns(&producer, &blockchain2, 3, 1);
    assert_eq!(blockchain2.read().block_number(), BLOCKS_PER_BATCH - 1);

    // Get the election block and corresponding history tree transactions from blockchain1.
    let blockchain = blockchain1.read();
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(BLOCKS_PER_EPOCH, true, None)
        .unwrap();

    let election_txs_1 = blockchain
        .history_store
        .prove_chunk(
            election_block_1.epoch_number(),
            election_block_1.block_number(),
            CHUNK_SIZE,
            0,
            None,
        )
        .unwrap();

    // Push the epoch to blockchain2.
    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        election_block_1.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    // Push blocks using history sync.
    assert_eq!(
        pusher.add_history_chunk(blockchain2.upgradable_read(), election_txs_1, 0, CHUNK_SIZE),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.head(), blockchain2.read().head());
}

// Tests if the history sync works when micro blocks have already been pushed in the blockchain,
// and tries to push history in multiple chunks
#[test]
fn history_sync_works_with_multiple_chunks() {
    // Produce macro blocks to complete one epoch in blockchain1.
    let env = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Build blockchain2.
    let env = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    let num_macro_blocks = BATCHES_PER_EPOCH as usize;
    let num_chunks = 4;
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(&producer, &blockchain1, num_macro_blocks, 2, 0);
    assert_eq!(blockchain1.read().block_number(), BLOCKS_PER_EPOCH);

    // Get the election block and corresponding history tree transactions from blockchain1.
    let blockchain = blockchain1.read();
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(BLOCKS_PER_EPOCH, true, None)
        .unwrap();

    let mut election_txs_1 = vec![];
    for i in 0..num_chunks {
        election_txs_1.push(
            blockchain
                .history_store
                .prove_chunk(
                    election_block_1.epoch_number(),
                    election_block_1.block_number(),
                    2 * BLOCKS_PER_EPOCH as usize / num_chunks,
                    i,
                    None,
                )
                .unwrap(),
        );
    }

    // Push the epoch to blockchain1.
    let mut pusher = Blockchain::start_history_sync(
        blockchain2.upgradable_read(),
        election_block_1.unwrap_macro(),
    )
    .expect("Failed starting history sync");

    let mut chunk_index = 0;
    for chunk in election_txs_1 {
        // Push blocks using history sync.
        assert_eq!(
            pusher.add_history_chunk(
                blockchain2.upgradable_read(),
                chunk,
                chunk_index,
                2 * BLOCKS_PER_EPOCH as usize / num_chunks
            ),
            Ok(PushResult::Extended)
        );
        chunk_index += 1;
    }

    assert_eq!(
        pusher.commit(blockchain2.upgradable_read()),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.head(), blockchain2.read().head());
}
