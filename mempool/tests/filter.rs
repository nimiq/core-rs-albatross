use std::convert::TryFrom;

use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_mempool::filter::{MempoolFilter, MempoolRules};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_test_log::test;
use nimiq_transaction::Transaction;

#[test]
fn it_can_blacklist_transactions() {
    let mut f: MempoolFilter = Default::default();

    let tx = Transaction::new_basic(
        Address::from([32u8; Address::SIZE]),
        Address::from([213u8; Address::SIZE]),
        Coin::try_from(100).unwrap(),
        Coin::try_from(1).unwrap(),
        123,
        NetworkId::Main,
    );

    let hash: Blake2bHash = tx.hash();
    f.blacklist(hash.clone());
    assert!(f.blacklisted(&hash));
    f.remove(&hash);
    assert!(!f.blacklisted(&hash));
}

#[test]
fn it_accepts_and_rejects_transactions() {
    let mut s = MempoolRules::default();
    s.tx_fee = Coin::try_from(1).unwrap();

    let f = MempoolFilter::new(s, MempoolFilter::DEFAULT_BLACKLIST_SIZE);

    let mut tx = Transaction::new_basic(
        Address::from([32u8; Address::SIZE]),
        Address::from([213u8; Address::SIZE]),
        Coin::try_from(0).unwrap(),
        Coin::try_from(0).unwrap(),
        0,
        NetworkId::Main,
    );

    assert!(!f.accepts_transaction(&tx));
    tx.fee = Coin::try_from(1).unwrap();
    assert!(f.accepts_transaction(&tx));
}

#[test]
fn it_has_a_limit() {
    let s = MempoolRules::default();
    let mut f = MempoolFilter::new(s, 1);

    let hash1: Blake2bHash = "hash1".hash();
    let hash2: Blake2bHash = "hash2".hash();

    assert!(!f.blacklisted(&hash1));
    assert!(!f.blacklisted(&hash2));

    f.blacklist(hash1.clone());

    assert!(f.blacklisted(&hash1));
    assert!(!f.blacklisted(&hash2));

    f.blacklist(hash2.clone());

    assert!(!f.blacklisted(&hash1));
    assert!(f.blacklisted(&hash2));

    f.remove(&hash1);

    assert!(!f.blacklisted(&hash1));
    assert!(f.blacklisted(&hash2));

    f.remove(&hash2);

    assert!(!f.blacklisted(&hash1));
    assert!(!f.blacklisted(&hash2));
}
