mod common;

use common::for_each_protocol_version;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::AccountType, networks::NetworkId, policy::upgrades, transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::{
        htlc_contract::{AnyHash, AnyHash32, AnyHash64},
        oracle_contract::{
            CreationTransactionData as OracleCreationData, IncomingOracleTransactionData,
        },
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

/// A bridge configured with `hash_function = Sha512` must be rejected at creation:
/// Sha512 has no `AnyMerkleProof` variant and `extract_burn_transaction_hash`
/// returns `UnsupportedHashFunction`, so such a bridge could accept deposits but
/// never process a burn-proof release — a permanent fund lock.
#[test]
fn bridge_creation_rejects_unsupported_hash_function() {
    let make_tx = |hash_function: AnyHash| {
        let data = BridgeCreationData {
            owner: Address::from(&key_pair().public),
            oracle_address: Address::from([4u8; 20]),
            source_chain_id: 1,
            chain_config: ChainConfig {
                chain_id: 1,
                hash_function,
                address_format: AddressFormat::Ethereum,
                endianness: Endianness::LittleEndian,
                block_time: std::time::Duration::from_secs(60),
                validation_program: ValidationProgram::empty(),
                max_proof_depth: 64,
            },
        };
        Transaction::new_contract_creation(
            Address::from([1u8; 20]),
            AccountType::Basic,
            vec![],
            AccountType::Bridge,
            data.serialize_to_vec(),
            1000.try_into().unwrap(),
            0.try_into().unwrap(),
            1,
            NetworkId::UnitAlbatross,
        )
    };

    // Sha512 is unsupported by the release path → rejected at an activated version.
    let sha512_tx = make_tx(AnyHash::Sha512(AnyHash64::from([0u8; 64])));
    assert_eq!(
        AccountType::verify_incoming_transaction(&sha512_tx, ACTIVATION),
        Err(TransactionError::InvalidData),
    );

    // The three supported hash functions are accepted at an activated version.
    for supported in [
        AnyHash::Blake2b(AnyHash32::from([0u8; 32])),
        AnyHash::Sha256(AnyHash32::from([0u8; 32])),
        AnyHash::Keccak256(AnyHash32::from([0u8; 32])),
    ] {
        let tx = make_tx(supported);
        assert_eq!(
            AccountType::verify_incoming_transaction(&tx, ACTIVATION),
            Ok(()),
        );
    }
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

// =====================================================================
// Oracle update signature integrity
// =====================================================================
//
// Authenticating an oracle `Update` takes two checks in two layers:
//
//   * this layer verifies the *signature* over the transaction content
//     (`IncomingOracleTransactionData::verify` -> `verify_transaction_signature`);
//   * `OracleContract::commit_incoming_transaction` then checks the signer *is the owner*
//     (`proof.is_signed_by(&self.owner)`, which only compares the derived address).
//
// Neither check is sufficient alone: without this one a valid owner signature could be lifted onto
// someone else's hashes; without the other, any well-signed update would be accepted. The owner
// half is covered by `it_rejects_update_from_non_owner` and friends in
// `primitives/account/tests/oracle_contract.rs`.

fn oracle_contract_address() -> Address {
    Address::from([1u8; 20])
}

fn any_hash(value: u8) -> AnyHash {
    AnyHash::Blake2b(AnyHash32::from([value; 32]))
}

/// Builds a signalling oracle `Update` carrying `hashes`. The signature is taken over the
/// transaction content with the proof field blanked, matching `verify_transaction_signature`.
fn oracle_update_tx(key: &KeyPair, hashes: Vec<AnyHash>) -> Transaction {
    let mut tx = unsigned_oracle_update_tx(hashes);
    let proof = SignatureProof::from_ed25519(key.public, key.sign(&tx.serialize_content()));
    tx.recipient_data =
        IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, proof)
            .expect("failed to set signature on data");
    tx
}

/// The same transaction with the default (all-zero) proof left in place.
fn unsigned_oracle_update_tx(hashes: Vec<AnyHash>) -> Transaction {
    let data = IncomingOracleTransactionData::Update {
        hashes,
        proof: SignatureProof::default(),
    };
    Transaction::new_signaling(
        oracle_contract_address(),
        AccountType::Oracle,
        oracle_contract_address(),
        AccountType::Oracle,
        0.try_into().unwrap(),
        data.serialize_to_vec(),
        1,
        NetworkId::UnitAlbatross,
    )
}

/// Control: a correctly signed update passes, so the rejections below cannot be vacuous.
#[test]
fn oracle_update_accepts_a_correctly_signed_update() {
    let tx = oracle_update_tx(&key_pair(), vec![any_hash(1), any_hash(2)]);
    assert_eq!(
        AccountType::verify_incoming_transaction(&tx, ACTIVATION),
        Ok(())
    );
}

/// A signature is bound to the hashes it was made over. Lifting a valid signature onto a
/// different set of hashes must fail — otherwise anyone who observed one legitimate update could
/// attest arbitrary Merkle roots, and every burn proof verified against them would be forgeable.
#[test]
fn oracle_update_rejects_signature_over_different_hashes() {
    let key = key_pair();

    // A legitimate update, and the proof it carries.
    let signed_tx = oracle_update_tx(&key, vec![any_hash(1)]);
    let lifted_proof = match IncomingOracleTransactionData::parse(&signed_tx).unwrap() {
        IncomingOracleTransactionData::Update { proof, .. } => proof,
        other => panic!("expected an Update, got {other:?}"),
    };
    assert!(
        lifted_proof.is_signed_by(&Address::from(&key.public)),
        "sanity: the lifted proof really is the owner's"
    );

    // The attacker's hashes, carrying the owner's signature.
    let mut forged_tx = unsigned_oracle_update_tx(vec![any_hash(0xEE)]);
    forged_tx.recipient_data = IncomingOracleTransactionData::set_signature_on_data(
        &forged_tx.recipient_data,
        lifted_proof,
    )
    .expect("failed to set signature on data");

    assert_eq!(
        AccountType::verify_incoming_transaction(&forged_tx, ACTIVATION),
        Err(TransactionError::InvalidProof),
    );
}

/// An update carrying the default, all-zero proof must be rejected here rather than relying on the
/// account layer's owner-address comparison to catch it. The default Ed25519 key is a known
/// verification-wildcard hazard, so this pins that it never satisfies signature verification.
#[test]
fn oracle_update_rejects_unsigned_update() {
    let tx = unsigned_oracle_update_tx(vec![any_hash(1)]);
    assert_eq!(
        AccountType::verify_incoming_transaction(&tx, ACTIVATION),
        Err(TransactionError::InvalidProof),
    );
}
