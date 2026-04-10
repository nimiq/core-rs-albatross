//! Regression tests for transaction validity window arithmetic.

use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_transaction::Transaction;

fn basic_transaction(validity_start_height: u32) -> Transaction {
    Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        Coin::try_from(100u64).unwrap(),
        Coin::try_from(0u64).unwrap(),
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
}

#[test]
fn is_valid_at_does_not_overflow_on_large_validity_start_height() {
    let window = Policy::transaction_validity_window_blocks();
    assert_eq!(
        window,
        Policy::transaction_validity_window() * Policy::blocks_per_batch()
    );

    // `overflow_start + window` would overflow without saturating arithmetic.
    let overflow_start = u32::MAX - (window - 1);
    let tx = basic_transaction(overflow_start);

    assert!(
        tx.is_valid_at(overflow_start),
        "is_valid_at({}) returned false for validity_start_height={}",
        overflow_start,
        overflow_start,
    );
}

#[test]
fn is_valid_at_does_not_overflow_at_u32_max() {
    let tx = basic_transaction(u32::MAX);

    assert!(!tx.is_valid_at(u32::MAX));
    assert!(tx.is_valid_at(u32::MAX - 1));
}

#[test]
fn is_valid_at_works_for_normal_values() {
    let tx = basic_transaction(1000);
    let window = Policy::transaction_validity_window_blocks();

    assert!(tx.is_valid_at(1000));
    assert!(tx.is_valid_at(1000 + window - Policy::blocks_per_batch() - 1));
    assert!(!tx.is_valid_at(1000 + window));
    assert!(!tx.is_valid_at(2000));
}
