use std::convert::{TryFrom, TryInto};

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_account::{
    Account, AccountError, AccountTransactionInteraction, AccountsTrie, VestingContract,
};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::{SignatureProof, Transaction, TransactionError, TransactionFlags};
use nimiq_trie::key_nibbles::KeyNibbles;

const CONTRACT: &str = "00002fbf9bd9c800fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a220000000000000001000000000003f480000002632e314a0000002fbf9bd9c800";

// This function is used to create the CONTRACT constant above. If you need to generate a new one just
// uncomment out the test flag.
//#[test]
#[allow(dead_code)]
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
    println!("{}", hex::encode(bytes));
    panic!();
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
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

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

    VestingContract::create(&accounts_tree, &mut db_txn, &transaction, 0, 0);

    match accounts_tree.get(
        &db_txn,
        &KeyNibbles::from(&transaction.contract_creation_address()),
    ) {
        Some(Account::Vesting(contract)) => {
            assert_eq!(contract.balance, 100.try_into().unwrap());
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.start_time, 0);
            assert_eq!(contract.time_step, 1000);
            assert_eq!(contract.step_amount, 100.try_into().unwrap());
            assert_eq!(contract.total_amount, 100.try_into().unwrap());
        }
        _ => panic!(),
    }

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();

    VestingContract::create(&accounts_tree, &mut db_txn, &transaction, 0, 0);

    match accounts_tree.get(
        &db_txn,
        &KeyNibbles::from(&transaction.contract_creation_address()),
    ) {
        Some(Account::Vesting(contract)) => {
            assert_eq!(contract.balance, 100.try_into().unwrap());
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.start_time, 0);
            assert_eq!(contract.time_step, 100);
            assert_eq!(contract.step_amount, 50.try_into().unwrap());
            assert_eq!(contract.total_amount, 100.try_into().unwrap());
        }
        _ => panic!(),
    }

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 32);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u64, &mut data);
    Serialize::serialize(&100u64, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    Serialize::serialize(&Coin::try_from(150).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();

    VestingContract::create(&accounts_tree, &mut db_txn, &transaction, 0, 0);

    match accounts_tree.get(
        &db_txn,
        &KeyNibbles::from(&transaction.contract_creation_address()),
    ) {
        Some(Account::Vesting(contract)) => {
            assert_eq!(contract.balance, 100.try_into().unwrap());
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.start_time, 0);
            assert_eq!(contract.time_step, 100);
            assert_eq!(contract.step_amount, 50.try_into().unwrap());
            assert_eq!(contract.total_amount, 150.try_into().unwrap());
        }
        _ => panic!(),
    }

    // Invalid data
    transaction.data = Vec::with_capacity(Address::SIZE + 2);
    Serialize::serialize(&owner, &mut transaction.data);
    Serialize::serialize(&0u16, &mut transaction.data);
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        VestingContract::create(&accounts_tree, &mut db_txn, &transaction, 0, 0,),
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        ))
    )
}

#[test]
fn it_does_not_support_incoming_transactions() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
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

    assert_eq!(
        VestingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 1, 2),
        Err(AccountError::InvalidForRecipient)
    );
    assert_eq!(
        VestingContract::revert_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 1, None),
        Err(AccountError::InvalidForRecipient)
    );
}

#[test]
fn it_can_verify_outgoing_transactions() {
    let sender_priv_key: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0").unwrap(),
    )
    .unwrap();

    let key_pair = KeyPair::from(sender_priv_key);
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
        Err(TransactionError::InvalidSerialization(
            SerializingError::InvalidValue
        ))
    );
}

#[test]
fn it_can_apply_and_revert_valid_transaction() {
    let sender_priv_key: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0").unwrap(),
    )
    .unwrap();
    let key_pair = KeyPair::from(sender_priv_key);

    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    let start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start_time: 0,
        time_step: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

    accounts_tree.put(
        &mut db_txn,
        &KeyNibbles::from(&[1u8; 20][..]),
        Account::Vesting(start_contract.clone()),
    );

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

    VestingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 200).unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[1u8; 20][..]))
            .unwrap()
            .balance(),
        800.try_into().unwrap()
    );
    VestingContract::revert_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 1, None)
        .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[1u8; 20][..]))
            .unwrap(),
        Account::Vesting(start_contract)
    );

    let start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start_time: 200,
        time_step: 0,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

    accounts_tree.put(
        &mut db_txn,
        &KeyNibbles::from(&[1u8; 20][..]),
        Account::Vesting(start_contract.clone()),
    );

    VestingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 200).unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[1u8; 20][..]))
            .unwrap()
            .balance(),
        800.try_into().unwrap()
    );
    VestingContract::revert_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 1, None)
        .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[1u8; 20][..]))
            .unwrap(),
        Account::Vesting(start_contract)
    );
}

#[test]
fn it_refuses_invalid_transaction() {
    let priv_key: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0").unwrap(),
    )
    .unwrap();
    let priv_key_alt: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("bd1cfcd49a81048c8c8d22a25766bd01bfa0f6b2eb0030f65241189393af96a2").unwrap(),
    )
    .unwrap();

    let key_pair = KeyPair::from(priv_key);
    let key_pair_alt = KeyPair::from(priv_key_alt);

    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    let start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start_time: 0,
        time_step: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

    accounts_tree.put(
        &mut db_txn,
        &KeyNibbles::from(&[1u8; 20][..]),
        Account::Vesting(start_contract),
    );

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

    assert_eq!(
        VestingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 200),
        Err(AccountError::InvalidSignature)
    );

    // Funds still vested
    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    assert_eq!(
        VestingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 100),
        Err(AccountError::InsufficientFunds {
            needed: 900.try_into().unwrap(),
            balance: 800.try_into().unwrap()
        })
    );
}
