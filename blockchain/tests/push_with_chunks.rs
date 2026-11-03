use nimiq_account::RevertInfo;
use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain_interface::{
    AbstractBlockchain, ChunksPushError, ChunksPushResult, PushError, PushResult,
};
use nimiq_genesis::NetworkId;
use nimiq_keys::{Address, KeyPair, PrivateKey, SecureGenerate};
use nimiq_primitives::{
    account::AccountError,
    coin::Coin,
    key_nibbles::KeyNibbles,
    policy::Policy,
    trie::{
        error::MerkleRadixTrieError,
        trie_chunk::{TrieChunkWithStart, TrieItem},
        trie_diff::TrieDiff,
    },
};
use nimiq_serde::Deserialize;
use nimiq_test_log::test;
use nimiq_test_utils::{block_production::TemporaryBlockProducer, test_rng};
use nimiq_transaction_builder::TransactionBuilder;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

macro_rules! check_invalid_chunk {
    ($f: expr, $pattern: pat_param) => {
        let result = push_invalid_chunk($f);
        let matches = matches!(result, $pattern);
        assert!(
            matches,
            "Invalid chunk did not produce expected result, got {:?}",
            result
        );
    };
}

fn key_pair_with_funds() -> KeyPair {
    let priv_key = PrivateKey::deserialize_from_vec(
        &hex::decode("6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587").unwrap(),
    )
    .unwrap();
    priv_key.into()
}

fn push_invalid_chunk<F>(f: F) -> Result<ChunksPushResult, ChunksPushError>
where
    F: FnOnce(&mut TrieChunkWithStart),
{
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    let block = temp_producer1.next_block(vec![], false);
    let valid_chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 1);
    let next_chunk_start = valid_chunk1.chunk.end_key.clone().unwrap();

    // Invalid keys end
    let mut invalid_chunk = temp_producer1.get_chunk(next_chunk_start.clone(), 2);
    f(&mut invalid_chunk);

    let valid_chunk2 = temp_producer1.get_chunk(next_chunk_start, 2);

    let (_, result) = temp_producer2
        .push_with_chunks(
            block,
            TrieDiff::default(),
            vec![valid_chunk1, invalid_chunk, valid_chunk2],
        )
        .expect("Block push should not fail");

    if let Err(ref e) = result {
        assert_eq!(e.chunk_index(), 1, "Should fail on second chunk");
    }

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

    result
}

#[test]
fn can_push_blocks_into_incomplete_trie() {
    let mut rng = test_rng(false);
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    // Block 1, 0 Chunks
    let key_pair = key_pair_with_funds();

    let keypair = KeyPair::generate(&mut rng);
    let address = Address::from(&keypair.public);

    let tx = TransactionBuilder::new_basic(
        &key_pair,
        address,
        100.try_into().unwrap(),
        Coin::ZERO,
        1 + Policy::genesis_block_number(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let (block, diff) = temp_producer1.next_block_and_diff_with_txs(vec![], false, vec![tx]);
    assert_eq!(
        temp_producer2.push_with_chunks(block, diff, vec![]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::EmptyChunks)))
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
}

#[test]
fn can_push_valid_chunks() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    // Block 1, 2 Chunks
    let block = temp_producer1.next_block(vec![], false);
    let chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 1);
    let chunk2 = temp_producer1.get_chunk(chunk1.chunk.end_key.clone().unwrap(), 1);
    let chunk3_start = chunk2.chunk.end_key.clone().unwrap();

    assert_eq!(
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![chunk1, chunk2]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(2, 0))))
    );

    // Block 2, 0 Chunks
    let block = temp_producer1.next_block(vec![], false);
    assert_eq!(
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::EmptyChunks)))
    );

    // Block 3, 1 Chunk
    let block = temp_producer1.next_block(vec![], false);
    let chunk3 = temp_producer1.get_chunk(chunk3_start, 3);

    assert_eq!(
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![chunk3]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 0))))
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
    let chunk5_start = chunk1.chunk.end_key.clone().unwrap();

    assert_eq!(
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![chunk1, chunk2]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 1))))
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
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![chunk3]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(0, 1))))
    );

    // Block 3, 2 Chunks, should skip chunk4
    let block = temp_producer1.next_block(vec![], false);
    let chunk4 = temp_producer1.get_chunk(KeyNibbles::BADBADBAD, 3);
    let chunk5 = temp_producer1.get_chunk(chunk5_start, 3);

    assert_eq!(
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![chunk4, chunk5]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 1))))
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
    let chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 1);
    let chunk2_start = chunk1.chunk.end_key.clone().unwrap();
    assert_eq!(
        temp_producer2.push_with_chunks(block1, TrieDiff::default(), vec![chunk1]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 0))))
    );

    let address_known = Address::from_hex("0000000000000000000000000000000000000000").unwrap();
    let address_unknown = Address::from_hex("f000000000000000000000000000000000000000").unwrap();
    assert!(
        KeyNibbles::from(&address_known) < chunk2_start,
        "Address should be in the known part of the trie"
    );
    assert!(
        KeyNibbles::from(&address_unknown) > chunk2_start,
        "Address should be in the unknown part of the trie"
    );

    let key_pair = key_pair_with_funds();
    let tx1 = TransactionBuilder::new_basic(
        &key_pair,
        address_known,
        100.try_into().unwrap(),
        Coin::ZERO,
        1 + Policy::genesis_block_number(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let tx2 = TransactionBuilder::new_basic(
        &key_pair,
        address_unknown,
        100.try_into().unwrap(),
        Coin::ZERO,
        1 + Policy::genesis_block_number(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    // Block 2b, 1 chunk (to be rebranched)
    let block2b = temp_producer1.next_block_no_push(vec![], true);

    // Block 2a, 1 chunk (to be reverted)
    let (block2a, diff2a) =
        temp_producer1.next_block_and_diff_with_txs(vec![], false, vec![tx1, tx2]);
    let chunk2a = temp_producer1.get_chunk(chunk2_start.clone(), 2);

    assert_eq!(
        temp_producer1.push(block2b.clone()),
        Ok(PushResult::Rebranched)
    );
    let diff2b = temp_producer1
        .blockchain
        .read()
        .chain_store
        .get_accounts_diff(&block2b.hash(), None)
        .unwrap();
    let chunk2b = temp_producer1.get_chunk(chunk2_start, 3);

    let block2a_number = block2a.block_number();
    assert_eq!(
        temp_producer2.push_with_chunks(block2a, diff2a.clone(), vec![chunk2a]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 0))))
    );

    {
        let rev_info = temp_producer2
            .blockchain
            .read()
            .chain_store
            .get_revert_info(block2a_number, None);
        match rev_info {
            Some(RevertInfo::Diff(diff)) => {
                assert_eq!(diff.0.len(), diff2a.0.len());
            }
            _ => panic!("Wrong revert info"),
        }
    }

    assert_eq!(
        temp_producer2.push_with_chunks(block2b, diff2b, vec![chunk2b]),
        Ok((PushResult::Rebranched, Ok(ChunksPushResult::Chunks(1, 0))))
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
    let chunk1 = temp_producer1.get_chunk(KeyNibbles::ROOT, 3);
    let chunk2_start = chunk1.chunk.end_key.clone().unwrap();
    assert_eq!(
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![chunk1]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 0))))
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
    let address_unknown = Address::from_hex("f000000000000000000000000000000000000000").unwrap();
    assert!(
        KeyNibbles::from(&address_known) < chunk2_start,
        "Address should be in the known part of the trie"
    );
    assert!(
        KeyNibbles::from(&address_unknown) > chunk2_start,
        "Address should be in the unknown part of the trie"
    );

    let tx1 = TransactionBuilder::new_basic(
        &key_pair,
        address_known,
        100.try_into().unwrap(),
        Coin::ZERO,
        1 + Policy::genesis_block_number(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let tx2 = TransactionBuilder::new_basic(
        &key_pair,
        address_unknown,
        100.try_into().unwrap(),
        Coin::ZERO,
        1 + Policy::genesis_block_number(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let (block, diff) = temp_producer1.next_block_and_diff_with_txs(vec![], false, vec![tx1, tx2]);
    assert_eq!(
        temp_producer2.push_with_chunks(block, diff, vec![]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::EmptyChunks)))
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
        temp_producer2.push_with_chunks(block, TrieDiff::default(), vec![chunk2]),
        Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 0))))
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
fn can_converge_with_changes_in_staking_contract() {
    let mut rng = ChaCha20Rng::seed_from_u64(0);
    let key_pair = key_pair_with_funds();
    let mut random_transactions = |height| -> Vec<_> {
        (0..10)
            .map(|_| {
                TransactionBuilder::new_create_staker(
                    &key_pair,
                    &KeyPair::generate(&mut rng),
                    None,
                    Policy::MINIMUM_STAKE.try_into().unwrap(),
                    Coin::ZERO,
                    height,
                    NetworkId::UnitAlbatross,
                )
                .unwrap()
            })
            .collect()
    };

    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new_incomplete();

    let mut chunk_start = KeyNibbles::ROOT;
    let mut i = 0;
    loop {
        let height = temp_producer1.blockchain.read().head().block_number();
        let (block, diff) =
            temp_producer1.next_block_and_diff_with_txs(vec![], false, random_transactions(height));
        let chunk = temp_producer1.get_chunk(chunk_start, 3);
        let chunk_end = chunk.chunk.end_key.clone();
        assert_eq!(
            temp_producer2.push_with_chunks(block, diff, vec![chunk]),
            Ok((PushResult::Extended, Ok(ChunksPushResult::Chunks(1, 0))))
        );
        println!("{i} {chunk_end:?}");
        i += 1;
        assert_eq!(
            temp_producer2
                .blockchain
                .read()
                .state
                .accounts
                .get_root_hash(None),
            temp_producer1
                .blockchain
                .read()
                .state
                .accounts
                .get_root_hash(None),
        );
        assert_eq!(
            temp_producer2
                .blockchain
                .read()
                .state
                .accounts
                .is_complete(None),
            chunk_end.is_none(),
        );
        match chunk_end {
            Some(key) => chunk_start = key,
            None => break,
        }
    }
}

#[test]
fn can_detect_invalid_chunks() {
    let mut rng = test_rng(false);
    // Chunks whose hash does not match (items are corrupted)
    check_invalid_chunk!(
        |invalid_chunk| invalid_chunk.chunk.items[1].value.push(1),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::ChunkHashMismatch)
        ))
    );

    // Chunks whose hash does not match (items are corrupted)
    let temp_producer1 = TemporaryBlockProducer::new();
    // Block 1, 0 Chunks
    let key_pair = key_pair_with_funds();

    let keypair = KeyPair::generate(&mut rng);
    let address = Address::from(&keypair.public);

    let tx = TransactionBuilder::new_basic(
        &key_pair,
        address,
        100.try_into().unwrap(),
        Coin::ZERO,
        1 + Policy::genesis_block_number(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    temp_producer1.next_block_with_txs(vec![], false, vec![tx]);

    check_invalid_chunk!(
        |invalid_chunk| *invalid_chunk =
            temp_producer1.get_chunk(invalid_chunk.start_key.clone(), 3),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );

    // Start key is after End key
    let address_unknown = Address::from_hex("1000000000000000000000000000000000000000").unwrap();
    let unknown_key = KeyNibbles::from(&address_unknown);
    check_invalid_chunk!(
        |invalid_chunk| invalid_chunk.chunk.end_key = Some(unknown_key),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );

    // Invalid keys end
    check_invalid_chunk!(
        |invalid_chunk| invalid_chunk.chunk.end_key = Some(KeyNibbles::BADBADBAD),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );

    // Ignore non-matching start key
    check_invalid_chunk!(
        |invalid_chunk| invalid_chunk.start_key = KeyNibbles::BADBADBAD,
        Ok(ChunksPushResult::Chunks(2, 1))
    );

    // First item before start key
    let address_known = Address::from_hex("1000000000000000000000000000000000000000").unwrap();
    let known_key = KeyNibbles::from(&address_known);
    check_invalid_chunk!(
        move |invalid_chunk| invalid_chunk
            .chunk
            .items
            .insert(0, TrieItem::new(known_key, vec![])),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );

    // Items after end key
    let address_unknown = Address::from_hex("ff00000000000000000000000000000000000000").unwrap();
    let unknown_key = KeyNibbles::from(&address_unknown);
    check_invalid_chunk!(
        move |invalid_chunk| invalid_chunk.chunk.end_key = Some(unknown_key),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );

    // Items not sorted
    check_invalid_chunk!(
        move |invalid_chunk| invalid_chunk.chunk.items.swap(0, 1),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );

    // Invalid end key (way later than the chunk size)
    let address_unknown = Address::from_hex("ff00000000000000000000000000000000000000").unwrap();
    let unknown_key = KeyNibbles::from(&address_unknown);
    check_invalid_chunk!(
        move |invalid_chunk| invalid_chunk.chunk.end_key = Some(unknown_key),
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );

    // Invalid end key (set to None)
    check_invalid_chunk!(
        |invalid_chunk| invalid_chunk.chunk.end_key = None,
        Err(ChunksPushError::AccountsError(
            1,
            AccountError::ChunkError(MerkleRadixTrieError::InvalidChunk(..))
        ))
    );
}

/// The block whose sibling the fork proposals build on: the last micro block of the second batch
/// (the first batch pays no rewards).
fn forked_block_number() -> u32 {
    Policy::macro_block_after(Policy::macro_block_after(Policy::genesis_block_number())) - 1
}

/// A node receives the trie items before `missing_from` with the first block and the rest of the
/// trie with `completing_block`. It stores a sibling of the forked block and verifies a macro block
/// proposal built on the sibling.
fn verify_proposal_on_fork(
    missing_from: KeyNibbles,
    completing_block: u32,
) -> Result<(), PushError> {
    let producer = TemporaryBlockProducer::new();
    let fork_producer = TemporaryBlockProducer::new();
    let node = TemporaryBlockProducer::new_incomplete();

    let forked_block = forked_block_number();
    assert!(completing_block <= forked_block);
    let push = |block: Block, diff: TrieDiff, chunks: Vec<TrieChunkWithStart>| {
        // The fork producer builds its own block at the forked height.
        if block.block_number() < forked_block {
            assert_eq!(fork_producer.push(block.clone()), Ok(PushResult::Extended));
        }
        let (result, chunks_result) = node.push_with_chunks(block, diff, chunks).unwrap();
        assert_eq!(result, PushResult::Extended);
        assert!(chunks_result.is_ok());
    };

    // First block with the trie items before `missing_from`, then empty blocks up to the
    // completing block.
    let (block, diff) = producer.next_block_and_diff_with_txs(vec![], false, vec![]);
    let items = producer.get_chunk(KeyNibbles::ROOT, 1000).chunk.items;
    let known_items = items.iter().filter(|item| item.key < missing_from).count();
    let chunks = match known_items {
        0 => vec![],
        n => vec![producer.get_chunk(KeyNibbles::ROOT, n)],
    };
    push(block, diff, chunks);

    while producer.blockchain.read().block_number() + 1 < completing_block {
        let (block, diff) = producer.next_block_and_diff_with_txs(vec![], false, vec![]);
        push(block, diff, vec![]);
    }
    let missing = node
        .blockchain
        .read()
        .get_missing_accounts_range(None)
        .unwrap();
    assert!(items
        .iter()
        .all(|item| missing.contains(&item.key) == (item.key >= missing_from)));

    // The completing block brings the rest of the trie.
    let (block, diff) = producer.next_block_and_diff_with_txs(vec![], false, vec![]);
    let chunk = producer.get_chunk(missing.start, 1000);
    push(block, diff, vec![chunk]);
    assert!(node.blockchain.read().accounts_complete());

    // Empty blocks up to the forked block, whose sibling is stored as a fork.
    while producer.blockchain.read().block_number() < forked_block {
        let (block, diff) = producer.next_block_and_diff_with_txs(vec![], false, vec![]);
        push(block, diff, vec![]);
    }
    let (sibling, sibling_diff) =
        fork_producer.next_block_and_diff_with_txs(b"fork".to_vec(), false, vec![]);
    assert_eq!(
        node.push_with_chunks(sibling, sibling_diff, vec![]),
        Ok((PushResult::Forked, Ok(ChunksPushResult::EmptyChunks)))
    );
    let head = node.blockchain.read().head_hash();

    // Proposal on top of the sibling, without its body.
    let header = {
        let blockchain = fork_producer.blockchain.read();
        let timestamp = blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME;
        fork_producer
            .producer
            .next_macro_block_proposal(&blockchain, timestamp, 0, vec![], None)
            .unwrap()
            .header
    };
    let proposal = MacroBlock {
        header,
        body: None,
        justification: None,
    };
    fork_producer
        .blockchain
        .upgradable_read()
        .verify_macro_block_proposal(proposal.clone(), 0, None)
        .expect("The proposal must verify on top of the sibling");

    let result = node
        .blockchain
        .upgradable_read()
        .verify_macro_block_proposal(proposal, 0, None)
        .map(|_| ());

    // Verification leaves the node's chain and state as they were.
    assert_eq!(node.blockchain.read().head_hash(), head);
    assert!(node.blockchain.read().accounts_complete());
    result
}

/// Verifying a proposal on a fork rebranches and reverts the chunk that completed the trie, here
/// the whole trie. The node cannot verify the proposal and must reject it.
#[test]
fn it_rejects_fork_proposals_if_the_rebranch_reverts_the_whole_trie() {
    let result = verify_proposal_on_fork(KeyNibbles::ROOT, forked_block_number());
    assert_eq!(result, Err(PushError::IncompleteAccountsTrie));
}

/// As above, but the reverted chunk starts at the validator's reward address, after the staking
/// contract and its data, which stay known.
#[test]
fn it_rejects_fork_proposals_if_the_rebranch_reverts_part_of_the_trie() {
    // The funded genesis account is the validator's reward address.
    let reward_address = KeyNibbles::from(&Address::from(&key_pair_with_funds().public));
    let staking_contract = KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS);
    assert!(staking_contract < reward_address && !staking_contract.is_prefix_of(&reward_address));

    let result = verify_proposal_on_fork(reward_address, forked_block_number());
    assert_eq!(result, Err(PushError::IncompleteAccountsTrie));
}

/// As in the whole-trie case, but the trie completes one block before the forked height: the
/// rebranch reverts no chunk and the node verifies the proposal.
#[test]
fn it_verifies_fork_proposals_if_the_rebranch_reverts_no_chunk() {
    let result = verify_proposal_on_fork(KeyNibbles::ROOT, forked_block_number() - 1);
    assert_eq!(result, Ok(()));
}
