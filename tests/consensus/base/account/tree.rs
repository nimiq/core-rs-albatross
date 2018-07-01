use core_rs::consensus::base::account::tree::AccountsTree;
use core_rs::consensus::base::account::{Account, BasicAccount};
use core_rs::consensus::base::primitive::Address;
use hex;

#[test]
fn it_can_put_and_get_a_balance() {
    let address = Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]);
    let mut account = Account::Basic(BasicAccount { balance: 20 });

    let mut tree = AccountsTree::new_volatile();

    // 1. Put account and check.
    tree.put(&address, account.clone());

    let account2 = tree.get(&address);
    assert!(account2.is_some());
    assert_eq!(account2.unwrap(), account);

    // 2. Increase balance, put, and check.
    if let Account::Basic(ref mut basic_account) = account {
        basic_account.balance = 50;
    }
    tree.put(&address, account.clone());

    let account2 = tree.get(&address);
    assert!(account2.is_some());
    assert_eq!(account2.unwrap(), account);

    // 3. Prune balance, put, and check.
    if let Account::Basic(ref mut basic_account) = account {
        basic_account.balance = 0;
    }
    tree.put(&address, account.clone());

    let account2 = tree.get(&address);
    assert!(account2.is_none());
}

#[test]
fn it_can_put_and_get_multiple_balances() {
    let address1 = Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]);
    let account1 = Account::Basic(BasicAccount { balance: 5 });
    let address2 = Address::from(&hex::decode("1000000000000000000000000000000000000000").unwrap()[..]);
    let account2 = Account::Basic(BasicAccount { balance: 55 });
    let address3 = Address::from(&hex::decode("1200000000000000000000000000000000000000").unwrap()[..]);
    let account3 = Account::Basic(BasicAccount { balance: 55555555 });

    let mut tree = AccountsTree::new_volatile();

    // Put accounts and check.
    tree.put(&address1, account1.clone());
    tree.put(&address2, account2.clone());
    tree.put(&address3, account3.clone());


    let account1a = tree.get(&address1);
    assert!(account1a.is_some());
    assert_eq!(account1a.unwrap(), account1);

    let account2a = tree.get(&address2);
    assert!(account2a.is_some());
    assert_eq!(account2a.unwrap(), account2);

    let account3a = tree.get(&address3);
    assert!(account3a.is_some());
    assert_eq!(account3a.unwrap(), account3);
}
