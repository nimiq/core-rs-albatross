mod common;

use common::for_each_protocol_version;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::AccountType, coin::Coin, networks::NetworkId, transaction::TransactionError,
};
use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use nimiq_transaction::{
    account::{
        vesting_contract::{CreationTransactionData, MAX_TIME_VALUE},
        AccountTransactionVerification,
    },
    SignatureProof, Transaction, TransactionFlags,
};

const OWNER_KEY: &str = "9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0";

fn key_pair() -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(OWNER_KEY).unwrap()).unwrap())
}

#[test]
#[allow(unused_must_use)]
fn it_can_verify_creation_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 8);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize_to_writer(&owner, &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);

    let mut transaction = Transaction::new_contract_creation(
        owner,
        AccountType::Basic,
        vec![],
        AccountType::Vesting,
        vec![],
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::UnitAlbatross,
    );

    // Invalid data
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction, v),
            Err(TransactionError::InvalidData),
        );
    });
    CreationTransactionData::parse_data(&data, transaction.value).unwrap();
    transaction.recipient_data = data;

    // Invalid recipient
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction, v),
            Err(TransactionError::InvalidForRecipient),
        );
    });
    transaction.recipient = transaction.contract_creation_address();

    // Valid
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction, v),
            Ok(()),
        );
    });

    // Invalid transaction flags
    transaction.flags = TransactionFlags::empty();
    transaction.recipient = transaction.contract_creation_address();
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction, v),
            Err(TransactionError::InvalidForRecipient),
        );
    });
    transaction.flags = TransactionFlags::CONTRACT_CREATION;

    // Valid
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize_to_writer(&sender, &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(100).unwrap(), &mut data);
    CreationTransactionData::parse_data(&data, transaction.value).unwrap();
    transaction.recipient_data = data;
    transaction.recipient = transaction.contract_creation_address();
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction, v),
            Ok(()),
        );
    });

    // Valid
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 32);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize_to_writer(&sender, &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&100u64.to_be_bytes(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(100).unwrap(), &mut data);
    Serialize::serialize_to_writer(&Coin::try_from(100).unwrap(), &mut data);
    CreationTransactionData::parse_data(&data, transaction.value).unwrap();
    transaction.recipient_data = data;
    transaction.recipient = transaction.contract_creation_address();
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction, v),
            Ok(()),
        );
    });

    // step amount > total amount
    let data = CreationTransactionData {
        owner: Address::from([0u8; 20]),
        start_time: 100,
        time_step: 0,
        step_amount: Coin::try_from(1000).unwrap(),
        total_amount: Coin::try_from(100).unwrap(),
    };
    transaction.recipient_data = data.to_tx_data();
    transaction.recipient = transaction.contract_creation_address();
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction, v),
            Err(TransactionError::InvalidData),
        );
    });
}

#[test]
fn it_rejects_out_of_bounds_time_values() {
    // `start_time` and `time_step` must be bounded to `MAX_TIME_VALUE` to avoid
    // arithmetic blow-ups in `VestingContract::min_cap`.

    let tx_value = Coin::try_from(100).unwrap();
    let owner = Address::from([0u8; 20]);

    let parse = |data: &CreationTransactionData| {
        CreationTransactionData::parse_data(&data.to_tx_data(), tx_value)
    };

    // 8-byte variant: only `time_step` is serialized (step_amount == total_amount, start_time == 0).
    let data = CreationTransactionData {
        owner: owner.clone(),
        start_time: 0,
        time_step: MAX_TIME_VALUE + 1,
        step_amount: tx_value,
        total_amount: tx_value,
    };
    assert!(matches!(parse(&data), Err(TransactionError::InvalidData)));

    // 24-byte variant: reject out-of-bound `start_time` (step_amount == total_amount).
    let data = CreationTransactionData {
        owner: owner.clone(),
        start_time: MAX_TIME_VALUE + 1,
        time_step: 100,
        step_amount: tx_value,
        total_amount: tx_value,
    };
    assert!(matches!(parse(&data), Err(TransactionError::InvalidData)));

    // 24-byte variant: reject out-of-bound `time_step`.
    let data = CreationTransactionData {
        owner: owner.clone(),
        start_time: 100,
        time_step: MAX_TIME_VALUE + 1,
        step_amount: tx_value,
        total_amount: tx_value,
    };
    assert!(matches!(parse(&data), Err(TransactionError::InvalidData)));

    // 32-byte variant: reject out-of-bound `start_time` (step_amount != total_amount).
    let data = CreationTransactionData {
        owner: owner.clone(),
        start_time: MAX_TIME_VALUE + 1,
        time_step: 100,
        step_amount: Coin::try_from(50).unwrap(),
        total_amount: tx_value,
    };
    assert!(matches!(parse(&data), Err(TransactionError::InvalidData)));

    // 32-byte variant: reject out-of-bound `time_step`.
    let data = CreationTransactionData {
        owner: owner.clone(),
        start_time: 100,
        time_step: MAX_TIME_VALUE + 1,
        step_amount: Coin::try_from(50).unwrap(),
        total_amount: tx_value,
    };
    assert!(matches!(parse(&data), Err(TransactionError::InvalidData)));

    // Boundary: `MAX_TIME_VALUE` itself is accepted.
    let data = CreationTransactionData {
        owner,
        start_time: MAX_TIME_VALUE,
        time_step: MAX_TIME_VALUE,
        step_amount: tx_value,
        total_amount: tx_value,
    };
    assert!(parse(&data).is_ok());
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

    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_outgoing_transaction(&tx, v),
            Err(TransactionError::InvalidSerialization(
                DeserializeError::unexpected_end()
            )),
        );
    });

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from_ed25519(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    for_each_protocol_version(|v| {
        assert_eq!(AccountType::verify_outgoing_transaction(&tx, v), Ok(()),);
    });

    tx.proof[22] = tx.proof[22] % 250 + 1;
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_outgoing_transaction(&tx, v),
            Err(TransactionError::InvalidProof),
        );
    });

    tx.proof[22] = tx.proof[22] % 251 + 3;
    // Proof is not a valid point, so Deserialize will result in an error.
    for_each_protocol_version(|v| {
        assert_eq!(
            AccountType::verify_outgoing_transaction(&tx, v),
            Err(TransactionError::InvalidProof),
        );
    });
}
