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
use nimiq_test_utils::{accounts_revert::TestCommitRevert, test_rng};
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
    let signature = key_1.sign(&tx.serialize_content());
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
    let contract = VestingContract::deserialize_from_vec(&hex::decode(CONTRACT).unwrap()).unwrap();
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
    let contract = VestingContract::deserialize_from_vec(&hex::decode(CONTRACT).unwrap()).unwrap();
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

    // Transaction 3: valid 32-byte format (total_amount <= tx_value)
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 32);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize_to_writer(&owner, &mut data);
    Serialize::serialize_to_writer(&0u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(50).unwrap(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(80).unwrap(), &mut data);
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
    assert_eq!(contract.total_amount, 80.try_into().unwrap());

    // Transaction 4: 32-byte format with total_amount > tx_value (rejected)
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 32);
    Serialize::serialize_to_writer(&owner, &mut data);
    Serialize::serialize_to_writer(&0u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(50).unwrap(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(150).unwrap(), &mut data);
    tx.recipient_data = data;
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
    );

    // Transaction 5: 32-byte format with step_amount > total_amount (rejected)
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 32);
    Serialize::serialize_to_writer(&owner, &mut data);
    Serialize::serialize_to_writer(&0u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(80).unwrap(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(50).unwrap(), &mut data);
    tx.recipient_data = data;
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
    );

    // Transaction 6: invalid data length
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

    let signature = key_1.sign(&tx.serialize_content());
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
    let signature = key_1_alt.sign(&tx.serialize_content());
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
    let signature = key_1.sign(&tx.serialize_content());
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
fn total_amount_exceeds_balance_panics_on_reserve_balance() {
    // Attack scenario: attacker uses the 32-byte vesting creation format to set
    // total_amount > transaction.value. This creates a contract where min_cap() can
    // exceed self.balance. When an outgoing transaction triggers can_change_balance(),
    // the error path computes `self.balance - min_cap` which panics on underflow.
    //
    // In the mempool path (reserve_balance), this runs inside a tokio::task::spawn,
    // so the panic kills only the verification task — not the whole node process.
    // However, in the block processing path (commit_outgoing_transaction), the panic
    // unwinds through Blockchain::push which holds an RwLockUpgradableReadGuard.
    // The poisoned lock makes the blockchain permanently unusable, effectively
    // crashing the node.

    let mut rng = test_rng(true);
    let key_owner = KeyPair::generate(&mut rng);
    let key_recipient = KeyPair::generate(&mut rng);

    // Create a vesting contract where total_amount (2000) > balance (1000).
    // This is reachable via the 32-byte CreationTransactionData format which
    // deserializes total_amount from attacker-controlled recipient_data without
    // validating total_amount <= tx_value.
    let vesting_contract = VestingContract {
        balance: Coin::from_u64_unchecked(1000),
        owner: Address::from(&key_owner.public),
        start_time: 0,
        time_step: u64::MAX, // Ensures 0 steps elapsed, so min_cap = total_amount
        step_amount: Coin::from_u64_unchecked(1),
        total_amount: Coin::from_u64_unchecked(2000), // > balance!
    };

    let accounts = TestCommitRevert::with_initial_state(&[
        (
            Address::from(&key_owner.public),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(1000),
            }),
        ),
        (
            Address([1u8; 20]),
            Account::Vesting(vesting_contract.clone()),
        ),
    ]);

    let mut db_txn = accounts.env().write_transaction();
    let sender_address = Address::from(&key_owner);
    let data_store = accounts.data_store(&sender_address);

    // Block time 0 with time_step=MAX means 0 steps have elapsed, so
    // min_cap = total_amount = 2000 > balance = 1000.
    let block_state = BlockState::new(1, 0);

    let mut reserved_balance = ReservedBalance::new(sender_address.clone());

    // Outgoing transaction for 1 luna — the amount doesn't matter,
    // any outgoing tx triggers the panic because min_cap > balance.
    let tx = make_signed_transaction(key_owner.clone(), key_recipient.clone(), 1);

    // This panics in can_change_balance at:
    //   balance: self.balance - min_cap  →  1000 - 2000  →  underflow panic
    //
    // After the fix, this should return Err(AccountError::InsufficientFunds)
    // without panicking.
    let result = vesting_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert!(
        result.is_err(),
        "Expected InsufficientFunds error, got: {:?}",
        result
    );
}

#[test]
fn total_amount_exceeds_balance_panics_on_commit_outgoing() {
    // Same root cause as above, but triggered via commit_outgoing_transaction —
    // the block processing path. This is the more dangerous path because it runs
    // while holding the blockchain RwLock, poisoning it on panic.

    let mut rng = test_rng(true);
    let key_owner = KeyPair::generate(&mut rng);
    let key_recipient = KeyPair::generate(&mut rng);

    let mut vesting_contract = VestingContract {
        balance: Coin::from_u64_unchecked(1000),
        owner: Address::from(&key_owner.public),
        start_time: 0,
        time_step: u64::MAX,
        step_amount: Coin::from_u64_unchecked(1),
        total_amount: Coin::from_u64_unchecked(2000),
    };

    let accounts = TestCommitRevert::with_initial_state(&[
        (
            Address::from(&key_owner.public),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(1000),
            }),
        ),
        (
            Address([1u8; 20]),
            Account::Vesting(vesting_contract.clone()),
        ),
    ]);

    let block_state = BlockState::new(1, 0);
    let tx = make_signed_transaction(key_owner.clone(), key_recipient.clone(), 1);

    // This panics in can_change_balance during commit_outgoing_transaction.
    // After the fix, this should return Err(AccountError::InsufficientFunds).
    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut vesting_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );
    assert!(
        result.is_err(),
        "Expected InsufficientFunds error, got: {:?}",
        result
    );
}

#[test]
fn max_start_time_does_not_panic_on_outgoing() {
    // `min_cap` must not panic for contracts with extreme `start_time`
    // values. Returns `InsufficientFunds` in both mempool and block paths.

    let mut rng = test_rng(true);
    let key_owner = KeyPair::generate(&mut rng);
    let key_recipient = KeyPair::generate(&mut rng);

    let mut vesting_contract = VestingContract {
        balance: Coin::from_u64_unchecked(1000),
        owner: Address::from(&key_owner.public),
        start_time: u64::MAX,
        time_step: 1,
        step_amount: Coin::from_u64_unchecked(1),
        total_amount: Coin::from_u64_unchecked(1000),
    };

    let accounts = TestCommitRevert::with_initial_state(&[
        (
            Address::from(&key_owner.public),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(1000),
            }),
        ),
        (
            Address([1u8; 20]),
            Account::Vesting(vesting_contract.clone()),
        ),
    ]);

    let block_state = BlockState::new(1, 1_700_000_000_000);
    let tx = make_signed_transaction(key_owner.clone(), key_recipient.clone(), 1);

    let sender_address = Address::from(&key_owner);
    let data_store = accounts.data_store(&sender_address);
    let mut reserved_balance = ReservedBalance::new(sender_address.clone());
    {
        let mut db_txn = accounts.env().write_transaction();
        let result = vesting_contract.reserve_balance(
            &tx,
            &mut reserved_balance,
            &block_state,
            data_store.read(&mut db_txn),
        );
        assert!(
            matches!(result, Err(AccountError::InsufficientFunds { .. })),
            "Expected InsufficientFunds, got: {:?}",
            result
        );
    }

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut vesting_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );
    assert!(
        matches!(result, Err(AccountError::InsufficientFunds { .. })),
        "Expected InsufficientFunds, got: {:?}",
        result
    );
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

    // Advancing the block state should allow further reserve balance.
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
