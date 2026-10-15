//! A release succeeds or is invalid, on a real chain.
//!
//! The bridge never pays for a failed release, so a block containing a release that fails is
//! invalid. The block producer drops a release that turns out stale while it builds a block, keeps
//! everything else, and puts releases ahead of oracle updates, so that an update overwriting the
//! slot a proof ends in cannot invalidate a release in the same block.

use nimiq_account::AccountsError;
use nimiq_block::{Block, MicroJustification};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_hash::Hash;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountError, coin::Coin, transaction::TransactionError};
use nimiq_test_log::test;
use nimiq_test_utils::bridge::{
    blake2b, burn_proof, funds, key, record_leaf, release_tx, BridgeChain, BRIDGE_DEPOSIT,
    TARGET_KEY,
};
use nimiq_transaction::{ExecutedTransaction, Transaction};
use nimiq_transaction_builder::TransactionBuilder;

const AMOUNT: u64 = 500;
const LOCK_AMOUNT: u64 = 1_000;

fn target() -> KeyPair {
    key(TARGET_KEY)
}

fn second_target() -> KeyPair {
    KeyPair::from(PrivateKey::from([0xBBu8; 32]))
}

fn address(key: &KeyPair) -> Address {
    Address::from(&key.public)
}

/// A plain transfer from the genesis account, standing in for every transaction that is not a
/// release.
fn transfer(chain: &BridgeChain, value: u64) -> Transaction {
    TransactionBuilder::new_basic(
        &funds(),
        Address::from([0x77u8; 20]),
        Coin::from_u64_unchecked(value),
        Coin::ZERO,
        chain.validity_start_height,
        chain.node.blockchain.read().network_id,
    )
    .unwrap()
}

/// A release of `burn`, proven against oracle index 0, that differs from `chain.release` only in
/// its validity start height: another transaction for the same burn, without a fee, which a failed
/// release used to land as.
fn another_release(chain: &BridgeChain, burn: Vec<u8>) -> Transaction {
    release_tx(
        &chain.bridge,
        &address(&target()),
        AMOUNT,
        burn_proof(burn, 0, &[]),
        &target(),
        chain.validity_start_height + 1,
    )
}

fn body(block: &Block) -> Vec<ExecutedTransaction> {
    block
        .unwrap_micro_ref()
        .body
        .as_ref()
        .unwrap()
        .transactions
        .clone()
}

fn ok(transactions: &[&Transaction]) -> Vec<ExecutedTransaction> {
    transactions
        .iter()
        .map(|transaction| ExecutedTransaction::Ok((*transaction).clone()))
        .collect()
}

/// `block`, a valid block with an empty body, carrying `transactions` instead and signed again by
/// the producer, as a producer that does not follow our rules could build it. Only the body
/// differs; the push fails at the release before it gets to the roots in the header.
fn with_body(chain: &BridgeChain, block: Block, transactions: Vec<ExecutedTransaction>) -> Block {
    let Block::Micro(mut block) = block else {
        panic!("expected a micro block");
    };
    let body = block.body.as_mut().unwrap();
    assert!(body.transactions.is_empty());
    body.transactions = transactions;
    block.header.body_root = body.hash();
    let signature = chain
        .node
        .producer
        .signing_key
        .sign(block.header.hash().as_slice());
    block.justification = Some(MicroJustification::Micro(signature));
    Block::Micro(block)
}

/// A release whose nonce is used fails. A block carrying it is invalid however it reports the
/// release's outcome: as succeeded, or as failed, which a failing release without a fee used to be.
/// The block is refused with the reason the release fails, and the bridge is never debited.
#[test]
fn a_block_containing_a_failing_release_is_rejected() {
    let chain = BridgeChain::new(10);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    chain.push_block(vec![chain.update(vec![record_leaf(&burn)])]);
    chain.push_block(vec![chain.release(&target(), AMOUNT, burn.clone(), 0, &[])]);
    assert_eq!(chain.luna(&chain.bridge), BRIDGE_DEPOSIT - AMOUNT);

    let stale = another_release(&chain, burn);
    let head = chain.node.blockchain.read().head_hash();

    for executed in [
        ExecutedTransaction::Ok(stale.clone()),
        ExecutedTransaction::Err(stale.clone()),
    ] {
        let block = with_body(&chain, chain.produce_block(vec![]), vec![executed]);
        assert_eq!(
            chain.node.push(block),
            Err(PushError::AccountsError(AccountsError::InvalidTransaction(
                AccountError::InvalidTransaction(TransactionError::InvalidData),
                stale.clone(),
            ))),
        );
        assert_eq!(chain.node.blockchain.read().head_hash(), head);
        assert_eq!(chain.luna(&chain.bridge), BRIDGE_DEPOSIT - AMOUNT);
    }

    // The chain goes on without it.
    chain.push_block(vec![]);
}

/// A release whose nonce a block already used is unappliable in the block being built. The producer
/// drops it and keeps every other transaction it picked, including releases and an oracle update.
#[test]
fn the_producer_drops_a_release_whose_nonce_is_used_and_keeps_the_rest() {
    let chain = BridgeChain::new(10);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    let other_burn = chain.burn(&address(&second_target()), AMOUNT, 1);
    chain.push_block(vec![
        chain.update(vec![record_leaf(&burn), record_leaf(&other_burn)])
    ]);
    chain.push_block(vec![chain.release(&target(), AMOUNT, burn.clone(), 0, &[])]);

    let stale = another_release(&chain, burn);
    let other_release = chain.release(&second_target(), AMOUNT, other_burn, 1, &[]);
    let transfer = transfer(&chain, 100);
    let lock = chain.lock(LOCK_AMOUNT);
    let update = chain.update(vec![blake2b(b"a later root")]);

    let block = chain.produce_block(vec![
        transfer.clone(),
        stale.clone(),
        lock.clone(),
        other_release.clone(),
        update.clone(),
    ]);
    assert_eq!(
        body(&block),
        ok(&[&other_release, &transfer, &lock, &update]),
        "only the stale release is dropped, and releases go first",
    );
    assert_eq!(chain.node.push(block), Ok(PushResult::Extended));
    assert_eq!(
        chain.luna(&chain.bridge),
        BRIDGE_DEPOSIT - 2 * AMOUNT + LOCK_AMOUNT
    );
}

/// A release proven against an oracle slot that an update has overwritten since is unappliable.
/// The producer drops it and keeps every other transaction, including the same burn's release
/// with its proof extended by the overwriting value.
#[test]
fn the_producer_drops_a_release_whose_slot_was_overwritten_and_keeps_the_rest() {
    // One slot, so every update overwrites the slot of the one before.
    let chain = BridgeChain::new(1);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    chain.push_block(vec![chain.update(vec![record_leaf(&burn)])]);
    let stale = chain.release(&target(), AMOUNT, burn.clone(), 0, &[]);

    let later_root = blake2b(b"a later root");
    chain.push_block(vec![chain.update(vec![later_root.clone()])]);
    let extended = chain.release_with_fee(&target(), AMOUNT, 1, burn, 0, &[later_root]);
    let transfer = transfer(&chain, 100);
    let lock = chain.lock(LOCK_AMOUNT);

    let block = chain.produce_block(vec![
        stale.clone(),
        transfer.clone(),
        extended.clone(),
        lock.clone(),
    ]);
    assert_eq!(
        body(&block),
        ok(&[&extended, &transfer, &lock]),
        "only the stale release is dropped",
    );
    assert_eq!(chain.node.push(block), Ok(PushResult::Extended));
    assert_eq!(chain.luna(&address(&target())), AMOUNT - 1);
}

/// An oracle update overwriting the slot a release's proof ends in would invalidate the release if
/// it ran first. The producer puts releases ahead of oracle updates, so both land in one block.
#[test]
fn a_release_placed_before_an_oracle_update_in_the_same_block_stays_valid() {
    let chain = BridgeChain::new(1);
    let burn = chain.burn(&address(&target()), AMOUNT, 1);
    chain.push_block(vec![chain.update(vec![record_leaf(&burn)])]);

    let overwrite = chain.update(vec![blake2b(b"a later root")]);
    let release = chain.release(&target(), AMOUNT, burn, 0, &[]);

    // Handed to the producer with the update first, as fee order may do.
    let block = chain.produce_block(vec![overwrite.clone(), release.clone()]);
    assert_eq!(body(&block), ok(&[&release, &overwrite]));
    assert_eq!(chain.node.push(block), Ok(PushResult::Extended));
    assert_eq!(chain.luna(&address(&target())), AMOUNT);
    assert_eq!(chain.luna(&chain.bridge), BRIDGE_DEPOSIT - AMOUNT);
}
