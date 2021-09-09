use std::convert::TryFrom;

use nimiq_account::{Accounts, Inherent, InherentType};
use nimiq_account::{Receipt, Receipts};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;
use nimiq_trie::key_nibbles::KeyNibbles;

#[test]
fn it_can_commit_and_revert_a_block_body() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_validator = Address::from([1u8; Address::SIZE]);

    let address_recipient = Address::from([2u8; Address::SIZE]);

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let mut receipts = vec![Receipt::Inherent {
        index: 0,
        pre_transactions: false,
        data: None,
    }];

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_validator), None),
        None
    );

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.commit(&mut txn, &[], &[reward.clone()], 1, 1),
        Ok(Receipts::from(receipts.clone()))
    );

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    let hash1 = accounts.get_root(None);

    let tx = Transaction::new_basic(
        address_validator.clone(),
        address_recipient.clone(),
        Coin::from_u64_unchecked(10),
        Coin::ZERO,
        1,
        NetworkId::Main,
    );

    let transactions = vec![tx];

    receipts.insert(
        0,
        Receipt::Transaction {
            index: 0,
            sender: false,
            data: None,
        },
    );

    receipts.insert(
        0,
        Receipt::Transaction {
            index: 0,
            sender: true,
            data: None,
        },
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.commit(&mut txn, &transactions, &[reward.clone()], 2, 2),
        Ok(Receipts::from(receipts.clone()))
    );

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10)
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000 + 10000 - 10)
    );

    assert_ne!(hash1, accounts.get_root(None));

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.revert(
            &mut txn,
            &transactions,
            &[reward],
            2,
            2,
            &Receipts::from(receipts)
        ),
        Ok(())
    );

    txn.commit();

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(hash1, accounts.get_root(None));
}

#[test]
fn it_correctly_rewards_validators() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_validator_1 = Address::from([1u8; Address::SIZE]);

    let address_validator_2 = Address::from([2u8; Address::SIZE]);

    let address_recipient_1 = Address::from([3u8; Address::SIZE]);

    let address_recipient_2 = Address::from([4u8; Address::SIZE]);

    // Validator 1 mines first block.
    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_validator_1), None),
        None
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_1.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts.commit(&mut txn, &[], &[reward], 1, 1).is_ok());

    txn.commit();

    // Create transactions to Recipient 1 and Recipient 2.
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator_1), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    let value1 = Coin::from_u64_unchecked(5);

    let fee1 = Coin::from_u64_unchecked(3);

    let value2 = Coin::from_u64_unchecked(7);

    let fee2 = Coin::from_u64_unchecked(11);

    let tx1 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_1.clone(),
        value1,
        fee1,
        2,
        NetworkId::Main,
    );

    let tx2 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_2.clone(),
        value2,
        fee2,
        2,
        NetworkId::Main,
    );

    // Validator 2 mines second block.
    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_validator_2), None),
        None
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_2.clone(),
        value: Coin::from_u64_unchecked(10000) + fee1 + fee2,
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &vec![tx1, tx2], &[reward], 2, 2)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator_1), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000) - value1 - fee1 - value2 - fee2
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator_2), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000) + fee1 + fee2
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient_1), None)
            .unwrap()
            .balance(),
        value1
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient_2), None)
            .unwrap()
            .balance(),
        value2
    );
}

#[test]
fn it_checks_for_sufficient_funds() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_sender = Address::from([1u8; Address::SIZE]);

    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut tx = Transaction::new_basic(
        address_sender.clone(),
        address_recipient.clone(),
        Coin::try_from(10).unwrap(),
        Coin::ZERO,
        1,
        NetworkId::Main,
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_sender.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let hash1 = accounts.get_root(None);

    assert_eq!(accounts.get(&KeyNibbles::from(&address_sender), None), None);

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    // Fails as address_sender does not have any funds.
    // Note: When the commit errors, we want to bracket the txn creation and the commit attempt.
    // Otherwise when we try to commit again, the test will get stuck.
    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 1, 1)
            .is_err());
    }

    assert_eq!(accounts.get(&KeyNibbles::from(&address_sender), None), None);

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(hash1, accounts.get_root(None));

    // Give address_sender one block reward.

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &[], &[reward.clone()], 1, 1)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    let hash2 = accounts.get_root(None);

    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    tx.value = Coin::from_u64_unchecked(1000000);

    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 2, 2)
            .is_err());
    }

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(hash2, accounts.get_root(None));

    // Multiple transactions exceeding funds.
    tx.value = Coin::from_u64_unchecked(5010);

    let mut tx2 = tx.clone();

    tx2.value += Coin::from_u64_unchecked(10);

    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &vec![tx, tx2], &[reward], 2, 2)
            .is_err());
    }

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(hash2, accounts.get_root(None));
}
