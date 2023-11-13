use std::convert::TryInto;

use nimiq_account::{
    Account, AccountTransactionInteraction, Accounts, BasicAccount, BlockLogger, BlockState, Log,
    ReservedBalance, TransactionLog, TransactionOperationReceipt, TransactionReceipt,
};
use nimiq_database::traits::Database;
use nimiq_keys::{Address, KeyPair, PrivateKey, SecureGenerate};
use nimiq_primitives::{account::AccountError, coin::Coin, networks::NetworkId};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::{
    accounts_revert::TestCommitRevert, test_rng::test_rng, transactions::TransactionsGenerator,
};
use nimiq_transaction::{SignatureProof, Transaction};

const SECRET_KEY_1: &str = "d0fbb3690f5308f457e245a3cc65ae8d6945155eadcac60d489ffc5583a60b9b";

fn init_tree() -> (TestCommitRevert, KeyPair, KeyPair) {
    let accounts = TestCommitRevert::new();
    let generator = TransactionsGenerator::new(
        Accounts::new(accounts.env.clone()),
        NetworkId::UnitAlbatross,
        test_rng(true),
    );

    let mut rng = test_rng(true);

    let key_1 = KeyPair::generate(&mut rng);
    generator.put_account(
        &Address::from(&key_1),
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    );

    let key_2 = KeyPair::generate(&mut rng);
    generator.put_account(
        &Address::from(&key_2),
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    );

    (accounts, key_1, key_2)
}

fn make_signed_transaction(value: u64, sender: Address, recipient: Address) -> Transaction {
    let mut tx = Transaction::new_basic(
        sender,
        recipient,
        value.try_into().unwrap(),
        1.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    let key_pair = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(SECRET_KEY_1).unwrap()).unwrap(),
    );

    let proof =
        SignatureProof::from_ed25519(key_pair.public, key_pair.sign(&tx.serialize_content()))
            .serialize_to_vec();

    tx.proof = proof;

    tx
}

#[test]
fn basic_transfer_works() {
    let (accounts, key_1, key_2) = init_tree();

    let sender_address = Address::from(&key_1);
    let recipient_address = Address::from(&key_2);

    let db_txn = accounts.env().write_transaction();
    let mut sender_account = accounts.get_complete(&sender_address, Some(&db_txn));
    let mut recipient_account = accounts.get_complete(&recipient_address, Some(&db_txn));
    drop(db_txn);

    // Works in the normal case.
    let tx = make_signed_transaction(100, sender_address.clone(), recipient_address.clone());

    let block_state = BlockState::new(1, 2);

    let mut tx_logger = TransactionLog::empty();
    let receipt = accounts
        .test_commit_outgoing_transaction(
            &mut sender_account,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .unwrap();

    assert_eq!(receipt, None);

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
        ],
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = accounts
        .test_commit_incoming_transaction(
            &mut recipient_account,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .unwrap();

    assert_eq!(receipt, None);
    assert!(tx_logger.logs.is_empty());

    assert_eq!(sender_account.balance(), Coin::from_u64_unchecked(899));
    assert_eq!(recipient_account.balance(), Coin::from_u64_unchecked(1100));

    // Doesn't work when the transaction value exceeds the account balance.
    let tx = make_signed_transaction(1000, sender_address.clone(), recipient_address.clone());

    assert_eq!(
        accounts.test_commit_outgoing_transaction(
            &mut sender_account,
            &tx,
            &block_state,
            &mut TransactionLog::empty(),
            true
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(1001),
            balance: Coin::from_u64_unchecked(899)
        })
    );

    // Doesn't work when the transaction total value exceeds the account balance.
    let tx = make_signed_transaction(899, sender_address, recipient_address);

    assert_eq!(
        accounts.test_commit_outgoing_transaction(
            &mut sender_account,
            &tx,
            &block_state,
            &mut TransactionLog::empty(),
            true
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(900),
            balance: Coin::from_u64_unchecked(899)
        })
    );
}

#[test]
fn create_and_prune_works() {
    let (accounts, key_1, _key_2) = init_tree();

    let sender_address = Address::from(&key_1);
    let recipient_address = Address([0; 20]);

    // Can create a new account and prune an empty account.
    let tx = make_signed_transaction(999, sender_address, recipient_address.clone());

    let block_state = BlockState::new(2, 2);
    let mut block_logger = BlockLogger::empty();

    // This tests the account pruning implicitly by asserting that the accounts root hash is back
    // to the initial state after reverting.
    let receipts = accounts
        .commit_and_test(&[tx], &[], &block_state, &mut block_logger)
        .unwrap();

    assert_eq!(
        receipts.transactions,
        vec![TransactionOperationReceipt::Ok(
            TransactionReceipt::default()
        )]
    );

    let db_txn = accounts.env().write_transaction();

    assert_eq!(
        accounts
            .get_complete(&recipient_address, Some(&db_txn))
            .balance(),
        Coin::from_u64_unchecked(999)
    );
}

#[test]
fn reserve_release_balance_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (accounts, key_1, key_2) = init_tree();

    let sender_address = Address::from(&key_1);
    let recipient_address = Address::from(&key_2);

    let mut db_txn = accounts.env().write_transaction();
    let sender_account = accounts.get_complete(&sender_address, Some(&db_txn));
    let data_store = accounts.data_store(&sender_address);

    let block_state = BlockState::new(1, 2);

    let mut reserved_balance = ReservedBalance::new(sender_address.clone());
    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the normal case.
    let tx = make_signed_transaction(100, sender_address.clone(), recipient_address.clone());

    let result = sender_account.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(101));
    assert!(result.is_ok());

    // Reserve the remaining
    let tx = make_signed_transaction(898, sender_address.clone(), recipient_address.clone());
    let result = sender_account.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert!(result.is_ok());

    // Doesn't work when there is not enough avl reserve.
    let tx = make_signed_transaction(1, sender_address.clone(), recipient_address.clone());
    let result = sender_account.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(1002),
            balance: Coin::from_u64_unchecked(1000)
        })
    );

    // Can release and reserve again.
    let tx = make_signed_transaction(100, sender_address.clone(), recipient_address.clone());
    let result =
        sender_account.release_balance(&tx, &mut reserved_balance, data_store.read(&mut db_txn));
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(899));
    assert!(result.is_ok());

    let tx = make_signed_transaction(100, sender_address.clone(), recipient_address.clone());
    let result = sender_account.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert!(result.is_ok());
}
