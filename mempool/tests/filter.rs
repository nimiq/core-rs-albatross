use nimiq_keys::Address;
use nimiq_primitives::networks::NetworkId;
use nimiq_mempool::filter::{MempoolFilter, Rules};
use nimiq_transaction::Transaction;
use nimiq_primitives::coin::Coin;

#[test]
fn it_can_blacklist_transactions() {
    let mut f: MempoolFilter = Default::default();

    let tx = Transaction::new_basic(
        Address::from([32u8; Address::SIZE]),
        Address::from([213u8; Address::SIZE]),
        Coin::from(100),
        Coin::from(1),
        123,
        NetworkId::Main,
    );

    f.blacklist(&tx);
    assert!(f.blacklisted(&tx));
    f.remove(&tx);
    assert!(!f.blacklisted(&tx));
}

#[test]
fn it_accepts_and_rejects_transactions() {
    let mut s: Rules = Rules::default();
    s.tx_fee = Coin::from(1);

    let f = MempoolFilter::new(s, MempoolFilter::DEFAULT_BLACKLIST_SIZE);

    let mut tx = Transaction::new_basic(
        Address::from([32u8; Address::SIZE]),
        Address::from([213u8; Address::SIZE]),
        Coin::from(0),
        Coin::from(0),
        0,
        NetworkId::Main,
    );

    assert!(!f.accepts_transaction(&tx));
    tx.fee = Coin::from(1);
    assert!(f.accepts_transaction(&tx));
}
