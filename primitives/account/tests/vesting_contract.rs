use std::convert::{TryFrom, TryInto};

use nimiq_account::{
    Account, AccountTransactionInteraction, BasicAccount, BlockState, Log, ReservedBalance,
    TransactionLog, VestingContract,
};
use nimiq_database::traits::Database;
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::{accounts_revert::TestCommitRevert, test_rng::test_rng};
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_utils::key_rng::SecureGenerate;

const CONTRACT: &str = "00002fbf9bd9c800fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a220000000000000001000000000003f480000002632e314a0000002fbf9bd9c800";

fn init_tree() -> (TestCommitRevert, VestingContract, KeyPair, KeyPair) {
    let mut rng = test_rng(true);
    let key_1 = KeyPair::generate(&mut rng);
    let key_2 = KeyPair::generate(&mut rng);
    let vesting_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_1.public),
        start_time: 0,
        time_step: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

    let accounts = TestCommitRevert::with_initial_state(&[
        (
            Address::from(&key_1.public),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(1000),
            }),
        ),
        (
            Address([1u8; 20]),
            Account::Vesting(vesting_contract.clone()),
        ),
    ]);

    (accounts, vesting_contract, key_1, key_2)
}

fn make_signed_transaction(key_1: KeyPair, key_2: KeyPair, value: u64) -> Transaction {
    let mut tx = Transaction::new_basic(
        Address::from(&key_1),
        Address::from(&key_2),
        Coin::from_u64_unchecked(value),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );
    tx.sender_type = AccountType::Vesting;
    let signature = key_1.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from_ed25519(key_1.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    tx
}

// This function is used to create the CONTRACT constant above.
#[test]
fn create_serialized_contract() {
    let contract = VestingContract {
        balance: Coin::from_u64_unchecked(52500000000000),
        owner: "fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22".parse().unwrap(),
        start_time: 1,
        time_step: 259200,
        step_amount: Coin::from_u64_unchecked(2625000000000),
        total_amount: Coin::from_u64_unchecked(52500000000000),
    };
    let mut bytes: Vec<u8> = Vec::with_capacity(contract.serialized_size());
    contract.serialize_to_writer(&mut bytes).unwrap();
    assert_eq!(CONTRACT, hex::encode(bytes));
}

#[test]
fn it_can_deserialize_a_vesting_contract() {
    let bytes: Vec<u8> = hex::decode(CONTRACT).unwrap();
    let contract: VestingContract = Deserialize::deserialize_from_vec(&bytes[..]).unwrap();
    assert_eq!(contract.balance, 52500000000000.try_into().unwrap());
    assert_eq!(
        contract.owner,
        "fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22".parse().unwrap()
    );
    assert_eq!(contract.start_time, 1);
    assert_eq!(contract.step_amount, 2625000000000.try_into().unwrap());
    assert_eq!(contract.time_step, 259200);
    assert_eq!(contract.total_amount, 52500000000000.try_into().unwrap());
}

#[test]
fn it_can_serialize_a_vesting_contract() {
    let bytes: Vec<u8> = hex::decode(CONTRACT).unwrap();
    let contract: VestingContract = Deserialize::deserialize_from_vec(&bytes[..]).unwrap();
    let mut bytes2: Vec<u8> = Vec::with_capacity(contract.serialized_size());
    let size = contract.serialize_to_writer(&mut bytes2).unwrap();
    assert_eq!(size, contract.serialized_size());
    assert_eq!(hex::encode(bytes2), CONTRACT);
}

#[test]
#[allow(unused_must_use)]
fn it_can_create_contract_from_transaction() {
    let (accounts, _vesting_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Transaction 1
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 8);
    let owner = Address::from(&key_1);
    Serialize::serialize_to_writer(&owner, &mut data);
    Serialize::serialize_to_writer(&1000u64.to_be_bytes(), &mut data);

    let mut tx = Transaction::new_contract_creation(
        owner.clone(),
        AccountType::Basic,
        vec![],
        AccountType::Vesting,
        data,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::UnitAlbatross,
    );

    // First contract creation
    let mut tx_logger = TransactionLog::empty();
    let contract = accounts
        .test_create_new_contract::<VestingContract>(
            &tx,
            Coin::ZERO,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to create contract");

    assert_eq!(
        tx_logger.logs,
        vec![Log::VestingCreate {
            contract_address: tx.contract_creation_address(),
            owner: owner.clone(),
            start_time: 0,
            time_step: 1000,
            step_amount: 100.try_into().unwrap(),
            total_amount: 100.try_into().unwrap()
        }]
    );

    let contract = match contract {
        Account::Vesting(contract) => contract,
        _ => panic!("Wrong account type created"),
    };

    assert_eq!(contract.balance, 100.try_into().unwrap());
    assert_eq!(contract.owner, owner);
    assert_eq!(contract.start_time, 0);
    assert_eq!(contract.time_step, 1000);
    assert_eq!(contract.step_amount, 100.try_into().unwrap());
    assert_eq!(contract.total_amount, 100.try_into().unwrap());

    // Transaction 2
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize_to_writer(&owner, &mut data);
    Serialize::serialize_to_writer(&0u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(50).unwrap(), &mut data);
    tx.recipient_data = data;
    tx.recipient = tx.contract_creation_address();

    let mut tx_logger = TransactionLog::empty();

    let contract = accounts
        .test_create_new_contract::<VestingContract>(
            &tx,
            Coin::ZERO,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to create contract");

    let contract = match contract {
        Account::Vesting(contract) => contract,
        _ => panic!("Wrong account type created"),
    };

    assert_eq!(contract.balance, 100.try_into().unwrap());
    assert_eq!(contract.owner, owner);
    assert_eq!(contract.start_time, 0);
    assert_eq!(contract.time_step, 100);
    assert_eq!(contract.step_amount, 50.try_into().unwrap());
    assert_eq!(contract.total_amount, 100.try_into().unwrap());

    // Transaction 3
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 32);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize_to_writer(&owner, &mut data);
    Serialize::serialize_to_writer(&0u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(50).unwrap(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(150).unwrap(), &mut data);
    tx.recipient_data = data;
    tx.recipient = tx.contract_creation_address();

    let mut tx_logger = TransactionLog::empty();
    let contract = accounts
        .test_create_new_contract::<VestingContract>(
            &tx,
            Coin::ZERO,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to create contract");

    let contract = match contract {
        Account::Vesting(contract) => contract,
        _ => panic!("Wrong account type created"),
    };

    assert_eq!(contract.balance, 100.try_into().unwrap());
    assert_eq!(contract.owner, owner);
    assert_eq!(contract.start_time, 0);
    assert_eq!(contract.time_step, 100);
    assert_eq!(contract.step_amount, 50.try_into().unwrap());
    assert_eq!(contract.total_amount, 150.try_into().unwrap());

    // Transaction 4: invalid data
    tx.recipient_data = Vec::with_capacity(Address::SIZE + 2);
    Serialize::serialize_to_writer(&owner, &mut tx.recipient_data);
    Serialize::serialize_to_writer(&0u16.to_be_bytes(), &mut tx.recipient_data);
    tx.recipient = tx.contract_creation_address();

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_create_new_contract::<VestingContract>(
        &tx,
        Coin::ZERO,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(
        result,
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        ))
    )
}

#[test]
fn it_does_not_support_incoming_transactions() {
    let (accounts, mut vesting_contract, key_1, key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    let mut tx = Transaction::new_basic(
        Address::from(&key_1),
        Address::from(&key_2),
        1.try_into().unwrap(),
        1000.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );
    tx.recipient_type = AccountType::Vesting;

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut vesting_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(tx_logger.logs.len(), 0);
    assert_eq!(result, Err(AccountError::InvalidForRecipient));
}

#[test]
fn it_can_apply_and_revert_valid_transaction() {
    let (accounts, mut vesting_contract, key_1, key_2) = init_tree();

    let block_state = BlockState::new(2, 200);

    let mut tx = Transaction::new_basic(
        Address::from(&key_1),
        Address::from(&key_2),
        200.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );
    tx.sender_type = AccountType::Vesting;

    let signature = key_1.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from_ed25519(key_1.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let _ = accounts
        .test_commit_outgoing_transaction(
            &mut vesting_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to commit transaction");

    assert_eq!(vesting_contract.balance, 800.try_into().unwrap());
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None
            }
        ]
    );

    let block_state = BlockState::new(3, 400);

    let mut tx_logger = TransactionLog::empty();
    let _ = accounts
        .test_commit_outgoing_transaction(
            &mut vesting_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to commit transaction");

    assert_eq!(vesting_contract.balance, 600.try_into().unwrap());
}

#[test]
fn it_refuses_invalid_transactions() {
    let (accounts, mut vesting_contract, key_1, key_1_alt) = init_tree();

    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        200.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );
    tx.sender_type = AccountType::Vesting;

    // Invalid signature
    let signature = key_1_alt.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from_ed25519(key_1_alt.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let block_state = BlockState::new(1, 200);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut vesting_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidSignature));
    assert_eq!(tx_logger.logs.len(), 0);

    // Funds still vested
    let signature = key_1.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from_ed25519(key_1.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let block_state = BlockState::new(100000, 100);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut vesting_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: 200.try_into().unwrap(),
            balance: 100.try_into().unwrap()
        })
    );
    assert_eq!(tx_logger.logs.len(), 0);
}

#[test]
fn reserve_release_balance_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (accounts, vesting_contract, key_1, key_2) = init_tree();
    let mut db_txn = accounts.env().write_transaction();
    let sender_address = Address::from(&key_1);
    let data_store = accounts.data_store(&sender_address);

    let block_state = BlockState::new(2, 200);

    let mut reserved_balance = ReservedBalance::new(sender_address.clone());
    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the normal case.
    let tx = make_signed_transaction(key_1.clone(), key_2.clone(), 190);
    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(190));
    assert!(result.is_ok());

    // Reserve the remaining
    let tx = make_signed_transaction(key_1.clone(), key_2.clone(), 10);
    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(200));
    assert!(result.is_ok());

    // Doesn't work when there is not enough avl reserve.
    let tx = make_signed_transaction(key_1.clone(), key_2.clone(), 1);
    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(200));
    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(201),
            balance: Coin::from_u64_unchecked(200)
        })
    );

    // Can release and reserve again.
    let tx = make_signed_transaction(key_1.clone(), key_2.clone(), 10);
    let result =
        vesting_contract.release_balance(&tx, &mut reserved_balance, data_store.read(&mut db_txn));
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(190));
    assert!(result.is_ok());

    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(200));
    assert!(result.is_ok());
}

#[test]
fn can_reserve_balance_after_time_step() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (accounts, vesting_contract, key_1, key_2) = init_tree();
    let mut db_txn = accounts.env().write_transaction();
    let sender_address = Address::from(&key_1);
    let data_store = accounts.data_store(&sender_address);

    let block_state = BlockState::new(2, 200);

    let mut reserved_balance = ReservedBalance::new(sender_address.clone());
    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the normal case.
    let tx = make_signed_transaction(key_1.clone(), key_2.clone(), 200);
    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(200));
    assert!(result.is_ok());

    // Doesn't work when there is not enough avl reserve.
    let tx = make_signed_transaction(key_1.clone(), key_2.clone(), 1);
    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(200));
    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(201),
            balance: Coin::from_u64_unchecked(200)
        })
    );

    // Advancing the block state should alow further reserve balance.
    let block_state = BlockState::new(3, 300);

    let tx = make_signed_transaction(key_1.clone(), key_2.clone(), 100);
    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(300));
    assert!(result.is_ok());

    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(300));
    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(400),
            balance: Coin::from_u64_unchecked(300)
        })
    );
}
