use beserial::Deserialize;
use nimiq_block::Block;
use nimiq_block_production::test_custom_block::{next_skip_block, BlockConfig};
use nimiq_blockchain::ChunksPushResult;
use nimiq_blockchain_interface::PushResult;
use nimiq_genesis::NetworkId;
use nimiq_keys::{Address, KeyPair, PrivateKey, SecureGenerate};
use nimiq_primitives::coin::Coin;
use nimiq_test_utils::block_production::TemporaryBlockProducer;
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_trie::key_nibbles::KeyNibbles;

fn key_pair_with_funds() -> KeyPair {
    let priv_key: PrivateKey = Deserialize::deserialize(
        &mut &hex::decode("6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587")
            .unwrap()[..],
    )
    .unwrap();
    priv_key.into()
}

#[test]
fn can_push_blocks_into_incomplete_trie() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    // Block 1, 0 Chunks
    let key_pair = key_pair_with_funds();

    let keypair = KeyPair::generate_default_csprng();
    let address = Address::from(&keypair.public);

    let tx = TransactionBuilder::new_basic(
        &key_pair,
        address.clone(),
        100.try_into().unwrap(),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let block = temp_producer1.next_block_with_txs(vec![], false, vec![tx]);
    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        temp_producer1
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None)
    );
}

#[test]
fn can_push_valid_chunks() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    // Block 1, 2 Chunks
    let block = temp_producer1.next_block(vec![], false);
    let chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 1);
    let chunk2 = temp_producer1.get_chunk(chunk1.chunk.keys_end.clone().unwrap(), 1);
    let chunk3_start = chunk2.chunk.keys_end.clone().unwrap();

    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![chunk1, chunk2]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    // Block 2, 0 Chunks
    let block = temp_producer1.next_block(vec![], false);
    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::NoChunks)))
    );

    // Block 3, 1 Chunk
    let block = temp_producer1.next_block(vec![], false);
    let chunk3 = temp_producer1.get_chunk(chunk3_start, 3);

    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![chunk3]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    // Done
    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .get_missing_accounts_range(None),
        None
    );
    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        temp_producer1
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None)
    );
}

#[test]
fn can_ignore_chunks_with_invalid_start_key() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    // Block 1, 2 Chunks, should skip chunk2
    let block = temp_producer1.next_block(vec![], false);
    let chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 2);
    let chunk2 = temp_producer1.get_chunk(KeyNibbles::ROOT, 3);
    let chunk5_start = chunk1.chunk.keys_end.clone().unwrap();

    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![chunk1, chunk2]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );
    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .get_missing_accounts_range(None),
        Some(chunk5_start.clone()..),
        "Should have discarded chunk 2"
    );

    // Block 2, 1 Chunk, should skip chunk3
    let block = temp_producer1.next_block(vec![], false);
    let chunk3 = temp_producer1.get_chunk(KeyNibbles::ROOT, 3);
    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![chunk3]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::NoChunks)))
    );

    // Block 3, 2 Chunks, should skip chunk4
    let block = temp_producer1.next_block(vec![], false);
    let chunk4 = temp_producer1.get_chunk(KeyNibbles::BADBADBAD, 3);
    let chunk5 = temp_producer1.get_chunk(chunk5_start, 3);

    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![chunk4, chunk5]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    // Done
    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .get_missing_accounts_range(None),
        None
    );
    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        temp_producer1
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None)
    );
}

#[test]
fn can_rebranch_and_revert_chunks() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    // Block 1, 1 chunk
    let block1 = temp_producer1.next_block(vec![], false);
    let chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 2);
    let chunk2_start = chunk1.chunk.keys_end.clone().unwrap();
    assert_eq!(
        temp_producer2.push_with_chunks(block1, vec![chunk1]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    // Block 2a, 1 chunk (to be reverted)
    let block2a = temp_producer1.next_block(vec![], false);
    let chunk2a = temp_producer1.get_chunk(chunk2_start.clone(), 2);

    // Block 2b, 1 chunk (to be rebranched)
    let block2b = Block::Micro({
        let blockchain = &temp_producer2.blockchain.read();
        next_skip_block(
            &temp_producer2.producer.voting_key,
            blockchain,
            &BlockConfig::default(),
        )
    });
    assert_eq!(
        temp_producer1.push(block2b.clone()),
        Ok(PushResult::Rebranched)
    );
    let chunk2b = temp_producer1.get_chunk(chunk2_start, 3);

    assert_eq!(
        temp_producer2.push_with_chunks(block2a, vec![chunk2a]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    assert_eq!(
        temp_producer2.push_with_chunks(block2b, vec![chunk2b]),
        Ok((PushResult::Rebranched, Ok(ChunksPushResult::Chunks)))
    );

    // Done
    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .get_missing_accounts_range(None),
        None
    );
    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        temp_producer1
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None)
    );
}

#[test]
fn can_partially_apply_blocks() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    // Block 1, 1 Chunks, 0 txs
    let key_pair = key_pair_with_funds();

    let block = temp_producer1.next_block_with_txs(vec![], false, vec![]);
    // Chunk covers trie until d...
    let chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 2);
    let chunk2_start = chunk1.chunk.keys_end.clone().unwrap();
    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![chunk1]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        temp_producer1
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None)
    );

    // Block 2, 0 Chunks, 2 txs
    let address_known = Address::from_hex("a000000000000000000000000000000000000000").unwrap();
    let address_unknown = Address::from_hex("e000000000000000000000000000000000000000").unwrap();
    let tx1 = TransactionBuilder::new_basic(
        &key_pair,
        address_known.clone(),
        100.try_into().unwrap(),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let tx2 = TransactionBuilder::new_basic(
        &key_pair,
        address_unknown.clone(),
        100.try_into().unwrap(),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let block = temp_producer1.next_block_with_txs(vec![], false, vec![tx1, tx2]);
    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::NoChunks)))
    );

    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash(None),
        None
    );

    // Block 3, 1 Chunks, 0 txs
    let block = temp_producer1.next_block_with_txs(vec![], false, vec![]);
    let chunk2 = temp_producer1.get_chunk(chunk2_start, 3);
    assert_eq!(
        temp_producer2.push_with_chunks(block, vec![chunk2]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks)))
    );

    assert_eq!(
        temp_producer2
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        temp_producer1
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None)
    );
}

// TODO Tests:
//
// Add invalid chunks
