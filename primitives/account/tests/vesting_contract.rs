use std::convert::{TryFrom, TryInto};

use nimiq_test_utils::{
    accounts_revert::TestCommitRevert, test_rng::test_rng, transactions::TransactionsGenerator,
};

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_account::{
    Account, Accounts, BasicAccount, BlockState, Log, TransactionLog, VestingContract,
};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    transaction::TransactionError,
};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::{vesting_contract::CreationTransactionData, AccountTransactionVerification},
    SignatureProof, Transaction, TransactionFlags,
};
use nimiq_utils::key_rng::SecureGenerate;

const CONTRACT: &str = "00002fbf9bd9c800fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a220000000000000001000000000003f480000002632e314a0000002fbf9bd9c800";
const OWNER_KEY: &str = "9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0";

fn key_pair() -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(OWNER_KEY).unwrap()).unwrap())
}

fn generate_contract(key_pair: &KeyPair) -> VestingContract {
    VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start_time: 0,
        time_step: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    }
}

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
        NetworkId::UnitAlbatross,
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

    // step amount > total amount
    let data = CreationTransactionData {
        owner: Address::from([0u8; 20]),
        start_time: 100,
        time_step: 0,
        step_amount: Coin::try_from(1000).unwrap(),
        total_amount: Coin::try_from(100).unwrap(),
    };
    transaction.data = data.serialize_to_vec();
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Ok(())
    );
}

#[test]
#[allow(unused_must_use)]
fn it_can_create_contract_from_transaction() {
    let (accounts, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Transaction 1
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 8);
    let owner = Address::from(&key_1);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&1000u64, &mut data);

    let mut tx = Transaction::new_contract_creation(
        data,
        owner.clone(),
        AccountType::Basic,
        AccountType::Vesting,
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
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    tx.data = data;
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
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    Serialize::serialize(&Coin::try_from(150).unwrap(), &mut data);
    tx.data = data;
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
    tx.data = Vec::with_capacity(Address::SIZE + 2);
    Serialize::serialize(&owner, &mut tx.data);
    Serialize::serialize(&0u16, &mut tx.data);
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
    let (accounts, key_1, key_2) = init_tree();

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

    let mut contract = generate_contract(&key_1);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(tx_logger.logs.len(), 0);
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
        NetworkId::UnitAlbatross,
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
    let (accounts, key_pair, key_2) = init_tree();

    let block_state = BlockState::new(2, 200);

    let start_contract = generate_contract(&key_pair);
    let mut contract = start_contract.clone();

    let mut tx = Transaction::new_basic(
        Address::from(&key_pair), // PITODO the new contract is not in the accounts
        Address::from(&key_2),
        200.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );
    tx.sender_type = AccountType::Vesting;

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let _ = accounts
        .test_commit_outgoing_transaction(&mut contract, &tx, &block_state, &mut tx_logger, true)
        .expect("Failed to commit transaction");

    assert_eq!(contract.balance, 800.try_into().unwrap());
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
        .test_commit_outgoing_transaction(&mut contract, &tx, &block_state, &mut tx_logger, true)
        .expect("Failed to commit transaction");

    assert_eq!(contract.balance, 600.try_into().unwrap());
}

#[test]
fn it_refuses_invalid_transactions() {
    let (accounts, key_pair, key_pair_alt) = init_tree();

    let mut contract = generate_contract(&key_pair);

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
    let signature = key_pair_alt.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair_alt.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let block_state = BlockState::new(1, 200);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidSignature));
    assert_eq!(tx_logger.logs.len(), 0);

    // Funds still vested
    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let block_state = BlockState::new(1, 100);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut contract,
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
