extern crate nimiq;

use nimiq::consensus::base::account::Accounts;
use nimiq::consensus::base::primitive::Address;
use nimiq::consensus::base::block::BlockBody;
use nimiq::utils::db::WriteTransaction;
use nimiq::utils::db::volatile::VolatileEnvironment;
use nimiq::consensus::base::transaction::Transaction;
use nimiq::consensus::networks::NetworkId;

#[test]
fn it_can_commit_a_block_body() {
    let env = VolatileEnvironment::new();
    let accounts = Accounts::new(&env);
    let address1 = Address::from([1u8; Address::SIZE]);

    let body1 = BlockBody {
        miner: address1.clone(),
        extra_data: Vec::new(),
        transactions: Vec::new(),
        pruned_accounts: Vec::new()
    };

    let mut txn = WriteTransaction::new(&env);
    assert_eq!(accounts.get(&address1, Some(&txn)).balance(), 0);
    assert!(accounts.commit_block_body(&mut txn, &body1, 1).is_ok());
    assert_eq!(accounts.get(&address1, Some(&txn)).balance(), 42);

    let address2 = Address::from([2u8; Address::SIZE]);
    let tx = Transaction::new_basic(
        address1.clone(),
        address2.clone(),
        10,
        0,
        1,
        NetworkId::Main
    );

    let body2 = BlockBody {
        miner: address1.clone(),
        extra_data: Vec::new(),
        transactions: vec![tx],
        pruned_accounts: Vec::new()
    };

    assert_eq!(accounts.get(&address2, Some(&txn)).balance(), 0);
    assert!(accounts.commit_block_body(&mut txn, &body2, 1).is_ok());
    assert_eq!(accounts.get(&address2, Some(&txn)).balance(), 10);
    assert_eq!(accounts.get(&address1, Some(&txn)).balance(), 74);
}

