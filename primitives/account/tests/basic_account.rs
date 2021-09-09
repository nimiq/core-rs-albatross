use std::convert::TryInto;

use beserial::{Deserialize, Serialize};
use nimiq_account::{
    Account, AccountError, AccountTransactionInteraction, AccountsTrie, BasicAccount,
};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::{SignatureProof, Transaction, TransactionError};
use nimiq_trie::key_nibbles::KeyNibbles;

const ADDRESS_1: &str = "83fa05dbe31f85e719f4c4fd67ebdba2e444d9f8";
const SECRET_KEY_1: &str = "d0fbb3690f5308f457e245a3cc65ae8d6945155eadcac60d489ffc5583a60b9b";
const ADDRESS_2: &str = "7182b1c2d0e2377d69413dc14c56cd923b67e41e";

#[test]
#[allow(unused_must_use)]
fn it_does_not_allow_creation() {
    let owner = Address::from([0u8; 20]);

    let transaction = Transaction::new_contract_creation(
        vec![],
        owner,
        AccountType::Basic,
        AccountType::Basic,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
}

#[test]
fn basic_transfer_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    init_tree(&accounts_tree, &mut db_txn);

    let address_recipient = Address::from_any_str(ADDRESS_2).unwrap();

    let key_sender = KeyNibbles::from(&Address::from_any_str(ADDRESS_1).unwrap());
    let key_recipient = KeyNibbles::from(&address_recipient);

    // Works in the normal case.
    let tx = make_signed_transaction(100, address_recipient.clone());

    assert_eq!(
        BasicAccount::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2),
        Ok(None)
    );

    assert_eq!(
        BasicAccount::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2),
        Ok(None)
    );

    assert_eq!(
        accounts_tree.get(&db_txn, &key_sender).unwrap().balance(),
        Coin::from_u64_unchecked(899)
    );

    assert_eq!(
        accounts_tree
            .get(&db_txn, &key_recipient)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(1100)
    );

    // Doesn't work when the transaction value exceeds the account balance.
    let tx = make_signed_transaction(1000, address_recipient.clone());

    assert_eq!(
        BasicAccount::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(1001),
            balance: Coin::from_u64_unchecked(899)
        })
    );

    // Doesn't work when the transaction total value exceeds the account balance.
    let tx = make_signed_transaction(899, address_recipient.clone());

    assert_eq!(
        BasicAccount::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(900),
            balance: Coin::from_u64_unchecked(899)
        })
    );

    // Can revert transaction.
    let tx = make_signed_transaction(100, address_recipient);

    assert_eq!(
        BasicAccount::revert_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2, None),
        Ok(())
    );

    assert_eq!(
        BasicAccount::revert_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2, None),
        Ok(())
    );

    assert_eq!(
        accounts_tree.get(&db_txn, &key_sender).unwrap().balance(),
        Coin::from_u64_unchecked(1000)
    );

    assert_eq!(
        accounts_tree
            .get(&db_txn, &key_recipient)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(1000)
    );
}

#[test]
fn create_and_prune_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    init_tree(&accounts_tree, &mut db_txn);

    let address_recipient = Address::from([0; 20]);

    let key_sender = KeyNibbles::from(&Address::from_any_str(ADDRESS_1).unwrap());
    let key_recipient = KeyNibbles::from(&address_recipient);

    // Can create a new account and prune an empty account.
    let tx = make_signed_transaction(999, address_recipient);

    assert_eq!(
        BasicAccount::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2),
        Ok(None)
    );

    assert_eq!(
        BasicAccount::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2),
        Ok(None)
    );

    assert_eq!(accounts_tree.get(&db_txn, &key_sender), None);

    assert_eq!(
        accounts_tree
            .get(&db_txn, &key_recipient)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(999)
    );

    // Can revert transaction.
    assert_eq!(
        BasicAccount::revert_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2, None),
        Ok(())
    );

    assert_eq!(
        BasicAccount::revert_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2, None),
        Ok(())
    );

    assert_eq!(
        accounts_tree.get(&db_txn, &key_sender).unwrap().balance(),
        Coin::from_u64_unchecked(1000)
    );

    assert_eq!(accounts_tree.get(&db_txn, &key_recipient), None);
}

fn init_tree(accounts_tree: &AccountsTrie, db_txn: &mut WriteTransaction) {
    let key_1 = KeyNibbles::from(&Address::from_any_str(ADDRESS_1).unwrap());
    let key_2 = KeyNibbles::from(&Address::from_any_str(ADDRESS_2).unwrap());

    accounts_tree.put(
        db_txn,
        &key_1,
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    );

    accounts_tree.put(
        db_txn,
        &key_2,
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    );
}

fn make_signed_transaction(value: u64, recipient: Address) -> Transaction {
    let sender = Address::from_any_str(ADDRESS_1).unwrap();

    let mut tx = Transaction::new_basic(
        sender,
        recipient,
        value.try_into().unwrap(),
        1.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    let key_pair = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(SECRET_KEY_1).unwrap()).unwrap(),
    );

    let proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()))
        .serialize_to_vec();

    tx.proof = proof;

    tx
}
