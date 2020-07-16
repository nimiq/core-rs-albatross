use std::convert::TryFrom;

use nimiq_block::Block;
use nimiq_blockchain::transaction_cache::TransactionCache;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_primitives::networks::NetworkInfo;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_transaction::Transaction;

#[test]
fn it_can_push_blocks() {
    let mut cache = TransactionCache::new();
    assert!(cache.is_empty());
    assert_eq!(cache.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW);

    let mut block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    let mut hash = block.header.hash::<Blake2bHash>();
    let tail_hash = hash.clone();

    cache.push_block(&block);
    assert!(!cache.is_empty());
    assert_eq!(
        cache.missing_blocks(),
        policy::TRANSACTION_VALIDITY_WINDOW - 1
    );
    assert_eq!(cache.head_hash(), hash);
    assert_eq!(cache.tail_hash(), hash);

    for i in 0..10 {
        assert!(block.body.as_ref().unwrap().transactions.is_empty() || cache.contains_any(&block));

        let mut b = block.clone();
        b.header.prev_hash = hash.clone();
        b.header.height = block.header.height + 1;

        let tx1 = Transaction::new_basic(
            [1u8; Address::SIZE].into(),
            [2u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 50).unwrap(),
            Coin::ZERO,
            b.header.height,
            NetworkId::Main,
        );
        let tx2 = Transaction::new_basic(
            [9u8; Address::SIZE].into(),
            [5u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 300).unwrap(),
            Coin::try_from(i).unwrap(),
            b.header.height,
            NetworkId::Main,
        );

        {
            let mut body = b.body.as_mut().unwrap();
            body.transactions = vec![tx1.clone(), tx2.clone()];
        }

        assert!(!cache.contains(&tx1.hash()));
        assert!(!cache.contains(&tx2.hash()));
        assert!(!cache.contains_any(&b));

        cache.push_block(&b);
        hash = b.header.hash();
        block = b;

        assert_eq!(
            cache.missing_blocks(),
            policy::TRANSACTION_VALIDITY_WINDOW - 2 - i as u32
        );
        assert_eq!(cache.head_hash(), hash);
        assert_eq!(cache.tail_hash(), tail_hash);

        assert!(cache.contains(&tx1.hash()));
        assert!(cache.contains(&tx2.hash()));
        assert!(cache.contains_any(&block));
    }
}

#[test]
fn it_can_revert_blocks() {
    let mut cache = TransactionCache::new();
    let mut block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    let mut hash = block.header.hash::<Blake2bHash>();
    cache.push_block(&block);

    let mut blocks = vec![block.clone()];
    for i in 0..10 {
        assert!(block.body.as_ref().unwrap().transactions.is_empty() || cache.contains_any(&block));

        let mut b = block.clone();
        b.header.prev_hash = hash.clone();
        b.header.height = block.header.height + 1;

        let tx1 = Transaction::new_basic(
            [1u8; Address::SIZE].into(),
            [2u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 50).unwrap(),
            Coin::ZERO,
            b.header.height,
            NetworkId::Main,
        );
        let tx2 = Transaction::new_basic(
            [9u8; Address::SIZE].into(),
            [5u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 300).unwrap(),
            Coin::try_from(i).unwrap(),
            b.header.height,
            NetworkId::Main,
        );

        {
            let mut body = b.body.as_mut().unwrap();
            body.transactions = vec![tx1.clone(), tx2.clone()];
        }

        assert!(!cache.contains(&tx1.hash()));
        assert!(!cache.contains(&tx2.hash()));
        assert!(!cache.contains_any(&b));

        cache.push_block(&b);
        hash = b.header.hash();
        block = b;
        blocks.push(block.clone());

        assert_eq!(
            cache.missing_blocks(),
            policy::TRANSACTION_VALIDITY_WINDOW - 2 - i as u32
        );
        assert_eq!(cache.head_hash(), hash);
        assert_eq!(cache.tail_hash(), blocks[0].header.hash());

        assert!(cache.contains(&tx1.hash()));
        assert!(cache.contains(&tx2.hash()));
        assert!(cache.contains_any(&block));
    }

    for i in 0..10 {
        assert!(cache.contains_any(&blocks[10 - i]));

        cache.revert_block(&blocks[10 - i]);

        assert_eq!(
            cache.missing_blocks(),
            policy::TRANSACTION_VALIDITY_WINDOW - 10 + i as u32
        );
        assert_eq!(cache.head_hash(), blocks[10 - i - 1].header.hash());
        assert_eq!(cache.tail_hash(), blocks[0].header.hash());

        assert!(!cache.contains_any(&blocks[10 - i]));
    }

    cache.revert_block(&blocks[0]);
    assert_eq!(cache.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW);
    assert!(cache.is_empty());
}

#[test]
fn it_removes_blocks_outside_the_validity_window() {
    let mut cache = TransactionCache::new();
    let mut block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    let mut hash = block.header.hash::<Blake2bHash>();
    cache.push_block(&block);

    let mut blocks = vec![block.clone()];
    for i in 0..(policy::TRANSACTION_VALIDITY_WINDOW + 25) as u64 {
        assert!(block.body.as_ref().unwrap().transactions.is_empty() || cache.contains_any(&block));

        let mut b = block.clone();
        b.header.prev_hash = hash.clone();
        b.header.height = block.header.height + 1;

        let tx1 = Transaction::new_basic(
            [1u8; Address::SIZE].into(),
            [2u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 50).unwrap(),
            Coin::ZERO,
            b.header.height,
            NetworkId::Main,
        );
        let tx2 = Transaction::new_basic(
            [9u8; Address::SIZE].into(),
            [5u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 300).unwrap(),
            Coin::try_from(i).unwrap(),
            b.header.height,
            NetworkId::Main,
        );

        {
            let mut body = b.body.as_mut().unwrap();
            body.transactions = vec![tx1.clone(), tx2.clone()];
        }

        assert!(!cache.contains(&tx1.hash()));
        assert!(!cache.contains(&tx2.hash()));
        assert!(!cache.contains_any(&b));

        cache.push_block(&b);
        hash = b.header.hash();
        block = b;
        blocks.push(block.clone());

        let tail_index =
            i.saturating_sub((policy::TRANSACTION_VALIDITY_WINDOW - 2) as u64) as usize;
        assert_eq!(
            cache.missing_blocks(),
            (policy::TRANSACTION_VALIDITY_WINDOW - 2).saturating_sub(i as u32)
        );
        assert_eq!(cache.head_hash(), hash);
        assert_eq!(cache.tail_hash(), blocks[tail_index].header.hash());

        assert!(cache.contains(&tx1.hash()));
        assert!(cache.contains(&tx2.hash()));
        assert!(cache.contains_any(&block));

        if i >= policy::TRANSACTION_VALIDITY_WINDOW as u64 {
            assert!(!cache.contains_any(&blocks[tail_index - 1]));
            assert!(cache.contains_any(&blocks[tail_index]));
        }
    }
}

#[test]
fn it_can_prepend_blocks() {
    let mut cache = TransactionCache::new();
    let mut block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    let mut hash = block.header.hash::<Blake2bHash>();

    let mut blocks = vec![block.clone()];
    for i in 0..(policy::TRANSACTION_VALIDITY_WINDOW - 1) as u64 {
        let mut b = block.clone();
        b.header.prev_hash = hash.clone();
        b.header.height = block.header.height + 1;

        let tx1 = Transaction::new_basic(
            [1u8; Address::SIZE].into(),
            [2u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 50).unwrap(),
            Coin::ZERO,
            b.header.height,
            NetworkId::Main,
        );
        let tx2 = Transaction::new_basic(
            [9u8; Address::SIZE].into(),
            [5u8; Address::SIZE].into(),
            Coin::try_from((i + 1) * 300).unwrap(),
            Coin::try_from(i).unwrap(),
            b.header.height,
            NetworkId::Main,
        );

        {
            let mut body = b.body.as_mut().unwrap();
            body.transactions = vec![tx1.clone(), tx2.clone()];
        }

        hash = b.header.hash();
        block = b;
        blocks.push(block.clone());
    }

    let mut i = 0;
    for b in blocks.iter().rev() {
        assert!(!cache.contains_any(&b));
        cache.prepend_block(&b);

        i += 1;
        assert_eq!(
            cache.missing_blocks(),
            policy::TRANSACTION_VALIDITY_WINDOW - i
        );
        assert_eq!(cache.head_hash(), blocks[blocks.len() - 1].header.hash());
        assert_eq!(cache.tail_hash(), b.header.hash());
        assert!(cache.contains_any(&b) || b.body.as_ref().unwrap().transactions.is_empty());
    }

    assert_eq!(cache.missing_blocks(), 0);

    for j in 0..10 {
        let b = &blocks[blocks.len() - j - 1];
        assert!(cache.contains_any(b));
        cache.revert_block(b);

        assert_eq!(cache.missing_blocks(), j as u32 + 1);
        assert_eq!(
            cache.head_hash(),
            blocks[blocks.len() - j - 2].header.hash()
        );
        assert_eq!(cache.tail_hash(), blocks[0].header.hash());
        assert!(!cache.contains_any(b));
    }
}
