use std::convert::TryInto;

use beserial::{Deserialize, Serialize};
use nimiq_account::{
    Account, AccountTransactionInteraction, AccountsTrie, BasicAccount, BlockState, DataStore,
    Receipts,
};
use nimiq_accounts_tree::Accounts;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    key_nibbles::KeyNibbles,
    networks::NetworkId,
    transaction::TransactionError,
};
use nimiq_test_log::test;
use nimiq_transaction::{account::AccountTransactionVerification, SignatureProof, Transaction};

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
    let accounts = Accounts::new(env.clone());

    let mut db_txn = WriteTransaction::new(&env);

    init_tree(&accounts.tree, &mut db_txn);

    let sender_address = Address::from_any_str(ADDRESS_1).unwrap();
    let recipient_address = Address::from_any_str(ADDRESS_2).unwrap();

    let mut sender_account = accounts.get_complete(&sender_address, Some(&db_txn));
    let mut recipient_account = accounts.get_complete(&recipient_address, Some(&db_txn));

    // Works in the normal case.
    let tx = make_signed_transaction(100, recipient_address.clone());

    let block_state = BlockState::new(1, 2);
    let sender_store = DataStore::new(&accounts.tree, &sender_address);

    let receipt = sender_account
        .commit_outgoing_transaction(&tx, &block_state, sender_store.write(&mut db_txn))
        .unwrap();

    assert_eq!(receipt, None);

    // assert_eq!(
    //     account_info.logs,
    //     vec![
    //         Log::PayFee {
    //             from: tx.sender.clone(),
    //             fee: tx.fee,
    //         },
    //         Log::Transfer {
    //             from: tx.sender.clone(),
    //             to: tx.recipient.clone(),
    //             amount: tx.value,
    //             data: None,
    //         },
    //     ],
    // );

    let recipient_store = DataStore::new(&accounts.tree, &recipient_address);

    let receipt = recipient_account
        .commit_incoming_transaction(&tx, &block_state, recipient_store.write(&mut db_txn))
        .unwrap();

    assert_eq!(receipt, None);
    // assert!(account_info.logs.is_empty());

    assert_eq!(sender_account.balance(), Coin::from_u64_unchecked(899));
    assert_eq!(recipient_account.balance(), Coin::from_u64_unchecked(1100));

    // Doesn't work when the transaction value exceeds the account balance.
    let tx = make_signed_transaction(1000, recipient_address.clone());

    assert_eq!(
        sender_account.commit_outgoing_transaction(
            &tx,
            &block_state,
            sender_store.write(&mut db_txn)
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(1001),
            balance: Coin::from_u64_unchecked(899)
        })
    );

    // Doesn't work when the transaction total value exceeds the account balance.
    let tx = make_signed_transaction(899, recipient_address.clone());

    assert_eq!(
        sender_account.commit_outgoing_transaction(
            &tx,
            &block_state,
            sender_store.write(&mut db_txn)
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(900),
            balance: Coin::from_u64_unchecked(899)
        })
    );

    // Can revert transaction.
    let tx = make_signed_transaction(100, recipient_address);

    assert_eq!(
        sender_account.revert_outgoing_transaction(
            &tx,
            &block_state,
            None,
            sender_store.write(&mut db_txn)
        ),
        Ok(()) // Ok(vec![
               //     Log::PayFee {
               //         from: tx.sender.clone(),
               //         fee: tx.fee,
               //     },
               //     Log::Transfer {
               //         from: tx.sender.clone(),
               //         to: tx.recipient.clone(),
               //         amount: tx.value,
               //         data: None,
               //     },
               // ])
    );

    assert_eq!(
        recipient_account.revert_incoming_transaction(
            &tx,
            &block_state,
            None,
            recipient_store.write(&mut db_txn)
        ),
        Ok(())
    );

    assert_eq!(sender_account.balance(), Coin::from_u64_unchecked(1000));
    assert_eq!(recipient_account.balance(), Coin::from_u64_unchecked(1000));
}

#[test]
fn create_and_prune_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let mut db_txn = WriteTransaction::new(&env);

    init_tree(&accounts.tree, &mut db_txn);

    let sender_address = Address::from_any_str(ADDRESS_1).unwrap();
    let recipient_address = Address::from([0; 20]);

    // Can create a new account and prune an empty account.
    let tx = make_signed_transaction(999, recipient_address.clone());

    let block_state = BlockState::new(2, 2);

    let receipts = accounts
        .commit(&mut db_txn, &[tx.clone()], &[], &block_state)
        .unwrap();

    assert_eq!(
        receipts.transactions,
        vec![TransactionOperationReceipt::Ok(
            TransactionReceipt::default()
        )]
    );

    assert_eq!(
        accounts.get_complete(&sender_address, Some(&db_txn)),
        Account::default()
    );

    assert_eq!(
        accounts
            .get_complete(&recipient_address, Some(&db_txn))
            .balance(),
        Coin::from_u64_unchecked(999)
    );

    // Can revert transaction.
    accounts
        .revert(&mut db_txn, &[tx], &[], &block_state, receipts)
        .unwrap();

    // assert_eq!(
    //     logs,
    //     vec![
    //         Log::PayFee {
    //             from: tx.sender.clone(),
    //             fee: tx.fee,
    //         },
    //         Log::Transfer {
    //             from: tx.sender.clone(),
    //             to: tx.recipient.clone(),
    //             amount: tx.value,
    //             data: None,
    //         },
    //     ]
    // );

    assert_eq!(
        accounts
            .get_complete(&sender_address, Some(&db_txn))
            .balance(),
        Coin::from_u64_unchecked(1000)
    );

    assert_eq!(
        accounts.get_complete(&recipient_address, Some(&db_txn)),
        Account::default()
    );
}

fn init_tree(tree: &AccountsTrie, db_txn: &mut WriteTransaction) {
    let key_1 = KeyNibbles::from(&Address::from_any_str(ADDRESS_1).unwrap());
    let key_2 = KeyNibbles::from(&Address::from_any_str(ADDRESS_2).unwrap());

    tree.put(
        db_txn,
        &key_1,
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    )
    .expect("complete trie");

    tree.put(
        db_txn,
        &key_2,
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    )
    .expect("complete trie");
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
