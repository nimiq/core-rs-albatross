mod common;

use common::for_each_protocol_version;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::AccountType, networks::NetworkId, policy::upgrades, transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::{
        htlc_contract::{AnyHash, AnyHash32},
        oracle_contract::CreationTransactionData as OracleCreationData,
        AccountTransactionVerification,
    },
    bridge_contract::{
        AddressFormat, ChainConfig, CreationTransactionData as BridgeCreationData, Endianness,
        ValidationProgram,
    },
    SignatureProof, Transaction,
};

const ACTIVATION: u16 = upgrades::v3::BRIDGE_ORACLE_CONTRACTS;

fn key_pair() -> KeyPair {
    KeyPair::from(
        PrivateKey::deserialize_from_vec(
            &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0")
                .unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn oracle_creation_is_version_gated() {
    let data = OracleCreationData {
        owner: Address::from(&key_pair().public),
        hash_count: 5,
    };

    let tx = Transaction::new_contract_creation(
        Address::from([1u8; 20]),
        AccountType::Basic,
        vec![],
        AccountType::Oracle,
        data.serialize_to_vec(),
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    for_each_protocol_version(|v| {
        let result = AccountType::verify_incoming_transaction(&tx, v);
        if v < ACTIVATION {
            assert_eq!(result, Err(TransactionError::InvalidForRecipient));
        } else {
            assert_eq!(result, Ok(()));
        }
    });
}

#[test]
fn oracle_outgoing_is_version_gated() {
    let key_pair = key_pair();

    let mut tx = Transaction::new_extended(
        Address::from([2u8; 20]),
        AccountType::Oracle,
        vec![],
        Address::from([3u8; 20]),
        AccountType::Basic,
        vec![],
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );
    let signature = key_pair.sign(&tx.serialize_content());
    tx.proof = SignatureProof::from_ed25519(key_pair.public, signature).serialize_to_vec();

    for_each_protocol_version(|v| {
        let result = AccountType::verify_outgoing_transaction(&tx, v);
        if v < ACTIVATION {
            assert_eq!(result, Err(TransactionError::InvalidForSender));
        } else {
            assert_eq!(result, Ok(()));
        }
    });
}

#[test]
fn bridge_creation_is_version_gated() {
    let data = BridgeCreationData {
        owner: Address::from(&key_pair().public),
        oracle_address: Address::from([4u8; 20]),
        source_chain_id: 1,
        chain_config: ChainConfig {
            chain_id: 1,
            hash_function: AnyHash::Keccak256(AnyHash32::from([0u8; 32])),
            address_format: AddressFormat::Ethereum,
            endianness: Endianness::LittleEndian,
            block_time: std::time::Duration::from_secs(60),
            validation_program: ValidationProgram::empty(),
            max_proof_depth: 64,
        },
    };

    let tx = Transaction::new_contract_creation(
        Address::from([1u8; 20]),
        AccountType::Basic,
        vec![],
        AccountType::Bridge,
        data.serialize_to_vec(),
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    for_each_protocol_version(|v| {
        let result = AccountType::verify_incoming_transaction(&tx, v);
        if v < ACTIVATION {
            assert_eq!(result, Err(TransactionError::InvalidForRecipient));
        } else {
            assert_eq!(result, Ok(()));
        }
    });
}

#[test]
fn bridge_incoming_transfer_is_version_gated() {
    // A regular transfer to a bridge contract (user locking funds).
    let tx = Transaction::new_extended(
        Address::from([1u8; 20]),
        AccountType::Basic,
        vec![],
        Address::from([2u8; 20]),
        AccountType::Bridge,
        vec![],
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    for_each_protocol_version(|v| {
        let result = AccountType::verify_incoming_transaction(&tx, v);
        if v < ACTIVATION {
            assert_eq!(result, Err(TransactionError::InvalidForRecipient));
        } else {
            assert_eq!(result, Ok(()));
        }
    });
}

#[test]
fn bridge_outgoing_is_version_gated() {
    // Empty sender_data fails to parse as OutgoingBridgeTransactionData, but
    // the version gate must fire before any parsing is attempted.
    let tx = Transaction::new_extended(
        Address::from([2u8; 20]),
        AccountType::Bridge,
        vec![],
        Address::from([3u8; 20]),
        AccountType::Basic,
        vec![],
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    for_each_protocol_version(|v| {
        let result = AccountType::verify_outgoing_transaction(&tx, v);
        if v < ACTIVATION {
            assert_eq!(result, Err(TransactionError::InvalidForSender));
        } else {
            assert!(matches!(
                result,
                Err(TransactionError::InvalidSerialization(_))
            ));
        }
    });
}
