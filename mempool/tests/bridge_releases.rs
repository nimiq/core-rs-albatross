//! Bridge releases in the mempool.
//!
//! A release that fails makes its block invalid, so the mempool admits a release only if it would
//! succeed against the current state, holds one release per target, and evicts a pending release
//! once a block uses its nonce or overwrites the oracle slot its proof ends in.

use std::sync::Arc;

use nimiq_block::Block;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mempool::{config::MempoolConfig, mempool::Mempool, verify::VerifyErr};
use nimiq_primitives::{account::AccountError, coin::Coin, transaction::TransactionError};
use nimiq_test_log::test;
use nimiq_test_utils::bridge::{blake2b, key, record_leaf, BridgeChain, TARGET_KEY};
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
