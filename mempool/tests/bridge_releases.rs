//! Bridge releases in the mempool.
//!
//! A release that fails makes its block invalid, so the mempool admits a release only if it would
//! succeed against the current state, holds one release per target, and evicts a pending release
//! once a block uses its nonce or overwrites the oracle slot its proof ends in.

use std::sync::Arc;

use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_database::traits::WriteTransaction;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mempool::{config::MempoolConfig, mempool::Mempool, verify::VerifyErr};
use nimiq_primitives::{
    account::AccountError, coin::Coin, key_nibbles::KeyNibbles, policy::upgrades,
    transaction::TransactionError,
};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    bridge::{blake2b, key, record_leaf, BridgeChain, TARGET_KEY},
};
use nimiq_transaction::Transaction;

const AMOUNT: u64 = 500;

fn target() -> KeyPair {
    key(TARGET_KEY)
}

fn second_target() -> KeyPair {
    KeyPair::from(PrivateKey::from([0xBBu8; 32]))
}

fn address(key: &KeyPair) -> Address {
    Address::from(&key.public)
}

fn mempool(chain: &BridgeChain) -> Mempool {
    Mempool::new(Arc::clone(&chain.node.blockchain), MempoolConfig::default())
}

/// Applies a block with `transactions` and tells the mempool about it, as consensus does.
fn adopt(chain: &BridgeChain, mempool: &Mempool, transactions: Vec<Transaction>) {
    let block: Block = chain.push_block(transactions);
    mempool.update(&[(block.hash(), block)], &[]);
}

fn pending(mempool: &Mempool) -> Vec<Transaction> {
    let mut transactions = mempool.get_transactions();
    transactions.sort();
    transactions
}

fn sorted(mut transactions: Vec<Transaction>) -> Vec<Transaction> {
    transactions.sort();
    transactions
}

/// A target can only release its next nonce, so the mempool holds one release per target: a second
/// release for the same target is refused, even a different transaction for the same burn, while
/// another target's release is admitted. A release that would fail is refused outright.
#[test]
fn the_mempool_refuses_a_second_release_per_target() {
    let chain = BridgeChain::new(10);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    let other_burn = chain.burn(&address(&second_target()), AMOUNT, 1);
    chain.push_block(vec![
        chain.update(vec![record_leaf(&burn), record_leaf(&other_burn)])
    ]);
    let mempool = mempool(&chain);

    let release = chain.release(&target(), AMOUNT, burn.clone(), 0, &[]);
    assert_eq!(mempool.add_transaction(release.clone(), None), Ok(()));

    let variant = chain.release_with_fee(&target(), AMOUNT, 10, burn, 0, &[]);
    assert_eq!(
        mempool.add_transaction(variant, None),
        Err(VerifyErr::InvalidAccount(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(2 * AMOUNT),
            balance: Coin::from_u64_unchecked(AMOUNT),
        })),
        "a second release for the same target must be refused",
    );

    let other_release = chain.release(&second_target(), AMOUNT, other_burn, 1, &[]);
    assert_eq!(mempool.add_transaction(other_release.clone(), None), Ok(()));

    // The target's next burn is not attested yet, so its release would fail.
    let next = chain.burn(&address(&target()), AMOUNT, 2);
    assert_eq!(
        mempool.add_transaction(chain.release(&target(), AMOUNT, next, 2, &[]), None),
        Err(VerifyErr::InvalidAccount(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        ))),
    );

    assert_eq!(pending(&mempool), sorted(vec![release, other_release]));
}

/// A pending release is evicted once a block uses its nonce, here through another release of the
/// same burn that reached the block producer first. Other targets' releases stay.
#[test]
fn the_mempool_evicts_a_release_once_its_nonce_is_used() {
    let chain = BridgeChain::new(10);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    let other_burn = chain.burn(&address(&second_target()), AMOUNT, 1);
    chain.push_block(vec![
        chain.update(vec![record_leaf(&burn), record_leaf(&other_burn)])
    ]);
    let mempool = mempool(&chain);

    let release = chain.release(&target(), AMOUNT, burn.clone(), 0, &[]);
    let other_release = chain.release(&second_target(), AMOUNT, other_burn, 1, &[]);
    assert_eq!(mempool.add_transaction(release, None), Ok(()));
    assert_eq!(mempool.add_transaction(other_release.clone(), None), Ok(()));

    let variant = chain.release_with_fee(&target(), AMOUNT, 10, burn, 0, &[]);
    adopt(&chain, &mempool, vec![variant]);

    assert_eq!(pending(&mempool), vec![other_release]);
}

/// A pending release is evicted once an oracle update overwrites the slot its proof ends in. The
/// same burn's release with its proof extended by the overwriting value is admitted in its place.
#[test]
fn the_mempool_evicts_a_release_once_its_slot_is_overwritten() {
    // One slot, so every update overwrites the slot of the one before.
    let chain = BridgeChain::new(1);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    chain.push_block(vec![chain.update(vec![record_leaf(&burn)])]);
    let mempool = mempool(&chain);

    let release = chain.release(&target(), AMOUNT, burn.clone(), 0, &[]);
    assert_eq!(mempool.add_transaction(release, None), Ok(()));

    let later_root = blake2b(b"a later root");
    adopt(
        &chain,
        &mempool,
        vec![chain.update(vec![later_root.clone()])],
    );
    assert_eq!(pending(&mempool), vec![]);

    let extended = chain.release(&target(), AMOUNT, burn, 0, &[later_root]);
    assert_eq!(mempool.add_transaction(extended.clone(), None), Ok(()));
    assert_eq!(pending(&mempool), vec![extended]);
}

// A node that is still syncing its accounts tree may hold the bridge but not every account or
// entry a release is checked against. It has to refuse the release, not panic: `pushTransaction`
// reaches the mempool without checking that consensus is established.

/// A bridge chain whose bridge sorts before its oracle in the accounts tree, so that a node syncing
/// the tree in key order can hold the bridge without the oracle.
fn chain_with_bridge_before_oracle() -> BridgeChain {
    (1..=64)
        .map(BridgeChain::new)
        .find(|chain| KeyNibbles::from(&chain.bridge) < KeyNibbles::from(&chain.oracle))
        .expect("some oracle size puts the bridge first")
}

/// A node that follows `chain` from genesis by block diffs, as live sync does after a reset, and has
/// received one chunk of the accounts tree, which ends at a key `is_end` accepts.
fn syncing_node(
    chain: &BridgeChain,
    is_end: impl Fn(&KeyNibbles) -> bool,
) -> TemporaryBlockProducer {
    let node =
        TemporaryBlockProducer::new_with_protocol_version(upgrades::v3::BRIDGE_ORACLE_CONTRACTS);
    {
        let blockchain = node.blockchain.read();
        let mut txn = blockchain.write_transaction();
        blockchain
            .state
            .accounts
            .reinitialize_as_incomplete(&mut (&mut txn).into());
        txn.commit();
    }

    let mut chunk = (1..=1_000)
        .map(|limit| chain.node.get_chunk(KeyNibbles::ROOT, limit))
        .find(|chunk| chunk.chunk.end_key.as_ref().is_some_and(&is_end));
    assert!(chunk.is_some(), "no chunk ends at an accepted key");

    let start = node.blockchain.read().block_number() + 1;
    let head = chain.node.blockchain.read().block_number();
    for number in start..=head {
        let (block, diff) = {
            let leader = chain.node.blockchain.read();
            let block = leader.chain_store.get_block_at(number, true, None).unwrap();
            let diff = leader
                .chain_store
                .get_accounts_diff(&block.hash(), None)
                .unwrap();
            (block, diff)
        };
        let chunks = if number == head {
            vec![chunk.take().unwrap()]
        } else {
            vec![]
        };
        let (pushed, chunks_pushed) = node.push_with_chunks(block, diff, chunks).unwrap();
        assert_eq!(pushed, PushResult::Extended);
        chunks_pushed.unwrap();
    }

    assert!(!node.blockchain.read().state.accounts.is_complete(None));
    assert!(node
        .blockchain
        .read()
        .get_account_if_complete(&chain.bridge)
        .is_some());
    node
}

/// A release of a burn nobody made, signed by `signer` for its own address. The record names the
/// bridge, so the release gets as far as reading the nonce store and the oracle.
fn forged_release(chain: &BridgeChain, signer: &KeyPair, nonce: u64) -> Transaction {
    let burn = chain.burn(&address(signer), AMOUNT, nonce);
    chain.release(signer, AMOUNT, burn, 0, &[])
}

/// A node that has synced the bridge but not its oracle cannot check a release, so it refuses it,
/// as a node with the whole tree refuses this one for its unwritten oracle index.
#[test]
fn a_syncing_node_refuses_a_release_whose_oracle_it_has_not_synced() {
    let chain = chain_with_bridge_before_oracle();
    let signer = KeyPair::from(PrivateKey::from([0xA7u8; 32]));

    assert_eq!(
        mempool(&chain).add_transaction(forged_release(&chain, &signer, 1), None),
        Err(VerifyErr::InvalidAccount(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        ))),
    );

    let bridge = KeyNibbles::from(&chain.bridge);
    let oracle = KeyNibbles::from(&chain.oracle);
    let node = syncing_node(&chain, |end| {
        bridge < *end && !bridge.is_prefix_of(end) && *end <= oracle
    });
    assert!(node
        .blockchain
        .read()
        .get_account_if_complete(&chain.oracle)
        .is_none());

    let mempool = Mempool::new(Arc::clone(&node.blockchain), MempoolConfig::default());
    assert_eq!(
        mempool.add_transaction(forged_release(&chain, &signer, 1), None),
        Err(VerifyErr::NoConsensus),
    );
}

/// A node whose synced part of the tree ends inside the bridge's nonce store cannot read the
/// target's last nonce, so it refuses the target's release.
#[test]
fn a_syncing_node_refuses_a_release_whose_nonce_it_has_not_synced() {
    let chain = BridgeChain::new(10);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    chain.push_block(vec![chain.update(vec![record_leaf(&burn)])]);
    chain.push_block(vec![chain.release(&target(), AMOUNT, burn, 0, &[])]);

    // The target's nonce is the only entry in the bridge's store, so a chunk ending inside the store
    // ends at it.
    let bridge = KeyNibbles::from(&chain.bridge);
    let node = syncing_node(&chain, |end| bridge < *end && bridge.is_prefix_of(end));

    let mempool = Mempool::new(Arc::clone(&node.blockchain), MempoolConfig::default());
    assert_eq!(
        mempool.add_transaction(forged_release(&chain, &target(), 2), None),
        Err(VerifyErr::NoConsensus),
    );
}
