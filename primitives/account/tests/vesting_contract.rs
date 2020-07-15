use std::convert::{TryFrom, TryInto};

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_account::{AccountError, AccountTransactionInteraction, AccountType, VestingContract};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::{SignatureProof, Transaction, TransactionError, TransactionFlags};

const CONTRACT: &str = "00002fbf9bd9c800fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22000000010003f480000002632e314a0000002fbf9bd9c800";

#[test]
fn it_can_deserialize_a_vesting_contract() {
    let bytes: Vec<u8> = hex::decode(CONTRACT).unwrap();
    let contract: VestingContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    assert_eq!(contract.balance, 52500000000000.try_into().unwrap());
    assert_eq!(
        contract.owner,
        Address::from("fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22")
    );
    assert_eq!(contract.start, 1);
    assert_eq!(contract.step_amount, 2625000000000.try_into().unwrap());
    assert_eq!(contract.step_blocks, 259200);
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
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 4);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&100u32, &mut data);

    let mut transaction = Transaction::new_contract_creation(
        vec![],
        owner.clone(),
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
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 16);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize(&sender, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&Coin::try_from(100).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Ok(())
    );

    // Valid
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize(&sender, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
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
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 4);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&1000u32, &mut data);

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
    match VestingContract::create(100.try_into().unwrap(), &transaction, 0) {
        Ok(contract) => {
            assert_eq!(contract.balance, 100.try_into().unwrap());
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.start, 0);
            assert_eq!(contract.step_blocks, 1000);
            assert_eq!(contract.step_amount, 100.try_into().unwrap());
            assert_eq!(contract.total_amount, 100.try_into().unwrap());
        }
        Err(_) => assert!(false),
    }

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 16);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    match VestingContract::create(100.try_into().unwrap(), &transaction, 0) {
        Ok(contract) => {
            assert_eq!(contract.balance, 100.try_into().unwrap());
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.start, 0);
            assert_eq!(contract.step_blocks, 100);
            assert_eq!(contract.step_amount, 50.try_into().unwrap());
            assert_eq!(contract.total_amount, 100.try_into().unwrap());
        }
        Err(_) => assert!(false),
    }

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&Coin::try_from(50).unwrap(), &mut data);
    Serialize::serialize(&Coin::try_from(150).unwrap(), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    match VestingContract::create(100.try_into().unwrap(), &transaction, 0) {
        Ok(contract) => {
            assert_eq!(contract.balance, 100.try_into().unwrap());
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.start, 0);
            assert_eq!(contract.step_blocks, 100);
            assert_eq!(contract.step_amount, 50.try_into().unwrap());
            assert_eq!(contract.total_amount, 150.try_into().unwrap());
        }
        Err(_) => assert!(false),
    }

    // Invalid data
    transaction.data = Vec::with_capacity(Address::SIZE + 2);
    Serialize::serialize(&owner, &mut transaction.data);
    Serialize::serialize(&0u16, &mut transaction.data);
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        VestingContract::create(0.try_into().unwrap(), &transaction, 0),
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        ))
    )
}

#[test]
fn it_does_not_support_incoming_transactions() {
    let mut contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from([1u8; 20]),
        start: 0,
        step_blocks: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

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
        VestingContract::check_incoming_transaction(&tx, 2),
        Err(AccountError::InvalidForRecipient)
    );
    assert_eq!(
        contract.commit_incoming_transaction(&tx, 2),
        Err(AccountError::InvalidForRecipient)
    );
    assert_eq!(
        contract.revert_incoming_transaction(&tx, 2, None),
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

    assert!(match AccountType::verify_outgoing_transaction(&tx) {
        Err(TransactionError::InvalidSerialization(SerializingError::IoError(_, _))) => true,
        _ => false,
    });

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
    let start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start: 0,
        step_blocks: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

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

    let mut contract = start_contract.clone();
    contract.commit_outgoing_transaction(&tx, 200).unwrap();
    assert_eq!(contract.balance, 800.try_into().unwrap());
    contract.revert_outgoing_transaction(&tx, 1, None).unwrap();
    assert_eq!(contract, start_contract);

    let start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start: 200,
        step_blocks: 0,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

    let mut contract = start_contract.clone();
    contract.commit_outgoing_transaction(&tx, 200).unwrap();
    assert_eq!(contract.balance, 800.try_into().unwrap());
    contract.revert_outgoing_transaction(&tx, 1, None).unwrap();
    assert_eq!(contract, start_contract);
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

    let mut start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start: 0,
        step_blocks: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

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
        start_contract.check_outgoing_transaction(&tx, 200),
        Err(AccountError::InvalidSignature)
    );
    assert_eq!(
        start_contract.commit_outgoing_transaction(&tx, 200),
        Err(AccountError::InvalidSignature)
    );

    // Funds still vested
    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    assert_eq!(
        start_contract.check_outgoing_transaction(&tx, 100),
        Err(AccountError::InsufficientFunds {
            needed: 900.try_into().unwrap(),
            balance: 800.try_into().unwrap(),
        })
    );
    assert_eq!(
        start_contract.commit_outgoing_transaction(&tx, 100),
        Err(AccountError::InsufficientFunds {
            needed: 900.try_into().unwrap(),
            balance: 800.try_into().unwrap(),
        })
    );
}
