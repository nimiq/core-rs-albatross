use std::convert::{TryFrom, TryInto};

use rand::thread_rng;

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_account::{Account, AccountTransactionInteraction, BlockState, VestingContract};
use nimiq_accounts_tree::Accounts;
use nimiq_database::{volatile::VolatileEnvironment, WriteTransaction};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    transaction::TransactionError,
};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::AccountTransactionVerification, SignatureProof, Transaction, TransactionFlags,
};
use nimiq_utils::key_rng::SecureGenerate;

const CONTRACT: &str = "00002fbf9bd9c800fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a220000000000000001000000000003f480000002632e314a0000002fbf9bd9c800";
const OWNER_KEY: &str = "9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0";

fn key_pair() -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(OWNER_KEY).unwrap()).unwrap())
}

fn generate_contract() -> VestingContract {
    let key_pair = key_pair();
    VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start_time: 0,
        time_step: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    }
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
    contract.serialize(&mut bytes).unwrap();
    assert_eq!(CONTRACT, hex::encode(bytes));
}

#[test]
fn it_can_deserialize_a_vesting_contract() {
    let bytes: Vec<u8> = hex::decode(CONTRACT).unwrap();
    let contract: VestingContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
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
    let contract: VestingContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    let mut bytes2: Vec<u8> = Vec::with_capacity(contract.serialized_size());
    let size = contract.serialize(&mut bytes2).unwrap();
    assert_eq!(size, contract.serialized_size());
    assert_eq!(hex::encode(bytes2), CONTRACT);
}

#[test]
#[allow(unused_must_use)]
fn it_can_verify_creation_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 8);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&100u64, &mut data);

    let mut transaction = Transaction::new_contract_creation(
        vec![],
        owner,
        AccountType::Basic,
        AccountType::Vesting,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    // Invalid data
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidData)
    );
    transaction.data = data;

    // Invalid recipient
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
    transaction.recipient = transaction.contract_creation_address();

    // Valid
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Ok(())
    );

    // Invalid transaction flags
    transaction.flags = TransactionFlags::empty();
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
    transaction.flags = TransactionFlags::CONTRACT_CREATION;

    // Valid
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize(&sender, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(100).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Ok(())
    );

    // Valid
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 32);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize(&sender, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(100).unwrap(), &mut data);
    Serialize::serialize(&Coin::try_from(100).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Ok(())
    );
}

#[test]
#[allow(unused_must_use)]
fn it_can_create_contract_from_transaction() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let block_state = BlockState::new(1, 1);
    let mut db_txn = WriteTransaction::new(&env);

    // Transaction 1
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 8);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&1000u64, &mut data);

    let mut transaction = Transaction::new_contract_creation(
        data,
        owner.clone(),
        AccountType::Basic,
        AccountType::Vesting,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    let data_store = accounts.data_store(&transaction.contract_creation_address());

    let contract = VestingContract::create_new_contract(
        &transaction,
        Coin::ZERO,
        &block_state,
        data_store.write(&mut db_txn),
    )
    .expect("Failed to create contract");

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
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();

    let data_store = accounts.data_store(&transaction.contract_creation_address());

    let contract = VestingContract::create_new_contract(
        &transaction,
        Coin::ZERO,
        &block_state,
        data_store.write(&mut db_txn),
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
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    Serialize::serialize(&Coin::try_from(150).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();

    let data_store = accounts.data_store(&transaction.contract_creation_address());

    let contract = VestingContract::create_new_contract(
        &transaction,
        Coin::ZERO,
        &block_state,
        data_store.write(&mut db_txn),
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
    transaction.data = Vec::with_capacity(Address::SIZE + 2);
    Serialize::serialize(&owner, &mut transaction.data);
    Serialize::serialize(&0u16, &mut transaction.data);
    transaction.recipient = transaction.contract_creation_address();

    let data_store = accounts.data_store(&transaction.contract_creation_address());

    let result = VestingContract::create_new_contract(
        &transaction,
        Coin::ZERO,
        &block_state,
        data_store.write(&mut db_txn),
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
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Address::from([2u8; 20]));
    let block_state = BlockState::new(1, 1);
    let mut db_txn = WriteTransaction::new(&env);

    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        1.try_into().unwrap(),
        1000.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.recipient_type = AccountType::Vesting;

    let mut contract = generate_contract();

    let result =
        contract.commit_incoming_transaction(&tx, &block_state, data_store.write(&mut db_txn));

    assert_eq!(result, Err(AccountError::InvalidForRecipient));

    let result = contract.revert_incoming_transaction(
        &tx,
        &block_state,
        None,
        data_store.write(&mut db_txn),
    );

    assert_eq!(result, Err(AccountError::InvalidForRecipient));
}

#[test]
fn it_can_verify_outgoing_transactions() {
    let key_pair = key_pair();
    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        1.try_into().unwrap(),
        1000.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.sender_type = AccountType::Vesting;

    assert!(matches!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            SerializingError::IoError(_)
        ))
    ));

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    tx.proof[22] = tx.proof[22] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );

    tx.proof[22] = tx.proof[22] % 251 + 3;
    // Proof is not a valid point, so Deserialize will result in an error.
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn it_can_apply_and_revert_valid_transaction() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Address::from([1u8; 20]));
    let block_state = BlockState::new(2, 200);
    let mut db_txn = WriteTransaction::new(&env);

    let key_pair = key_pair();
    let start_contract = generate_contract();
    let mut contract = start_contract.clone();

    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        200.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.sender_type = AccountType::Vesting;

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let receipt = contract
        .commit_outgoing_transaction(&tx, &block_state, data_store.write(&mut db_txn))
        .expect("Failed to commit transaction");

    assert_eq!(contract.balance, 800.try_into().unwrap());

    contract
        .revert_outgoing_transaction(&tx, &block_state, receipt, data_store.write(&mut db_txn))
        .expect("Failed to revert transaction");

    assert_eq!(contract, start_contract);

    let block_state = BlockState::new(1, 200);

    let receipt = contract
        .commit_outgoing_transaction(&tx, &block_state, data_store.write(&mut db_txn))
        .expect("Failed to commit transaction");

    assert_eq!(contract.balance, 800.try_into().unwrap());

    contract
        .revert_outgoing_transaction(&tx, &block_state, receipt, data_store.write(&mut db_txn))
        .expect("Failed to revert transaction");

    assert_eq!(contract, start_contract);
}

#[test]
fn it_refuses_invalid_transactions() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Address::from([1u8; 20]));
    let mut db_txn = WriteTransaction::new(&env);

    let key_pair = key_pair();
    let key_pair_alt = KeyPair::generate(&mut thread_rng());
    let mut contract = generate_contract();

    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        200.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.sender_type = AccountType::Vesting;

    // Invalid signature
    let signature = key_pair_alt.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair_alt.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let block_state = BlockState::new(1, 200);

    let result =
        contract.commit_outgoing_transaction(&tx, &block_state, data_store.write(&mut db_txn));

    assert_eq!(result, Err(AccountError::InvalidSignature));

    // Funds still vested
    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let block_state = BlockState::new(1, 100);

    let result =
        contract.commit_outgoing_transaction(&tx, &block_state, data_store.write(&mut db_txn));

    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: 200.try_into().unwrap(),
            balance: 100.try_into().unwrap()
        })
    );
}
