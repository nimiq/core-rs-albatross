use nimiq_hash::{Blake2bHash, HashOutput, Hasher, Keccak256Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::AccountType, coin::Coin, networks::NetworkId, policy::upgrades,
    transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::{
        bridge_contract::{AnyMerkleProof, OutgoingBridgeTransactionData, OutgoingTransaction},
        htlc_contract::{AnyHash, AnyHash32},
        oracle_contract::IncomingOracleTransactionData,
    },
    SignatureProof, Transaction, TransactionFlags,
};
use nimiq_transaction_builder::{TransactionBuilder, TransactionBuilderError};
use nimiq_utils::merkle::MerkleProof;

const OWNER_PRIVATE_KEY: &str = "b410a7a583cbc13ef4f1cbddace30928bcb4f9c13722414bc4a2faaba3f4e187";
const PAYER_PRIVATE_KEY: &str = "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";
const CONTRACT_ADDRESS: &str = "NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812";
const RECIPIENT_ADDRESS: &str = "NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62";

const NETWORK_ID: NetworkId = NetworkId::UnitAlbatross;
const PROTOCOL_VERSION: u16 = upgrades::v3::BRIDGE_ORACLE_CONTRACTS;

fn key_pair(private_key: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(private_key).unwrap()).unwrap())
}

fn contract_address() -> Address {
    Address::from_any_str(CONTRACT_ADDRESS).unwrap()
}

fn recipient_address() -> Address {
    Address::from_any_str(RECIPIENT_ADDRESS).unwrap()
}

fn keccak_hash(data: &[u8]) -> AnyHash {
    AnyHash::Keccak256(AnyHash32::from(
        Keccak256Hasher::default().digest(data).as_bytes(),
    ))
}

fn sign(key_pair: &KeyPair, tx: &Transaction) -> SignatureProof {
    SignatureProof::from_ed25519(key_pair.public, key_pair.sign(&tx.serialize_content()))
}

/// Builds an oracle signaling transaction by hand: the owner signs the content with a default
/// proof in the data, then the payer signs the content with the owner's proof embedded.
fn make_oracle_signaling_transaction(
    data: IncomingOracleTransactionData,
    owner: &KeyPair,
    payer: &KeyPair,
) -> Transaction {
    let mut tx = Transaction::new_signaling(
        Address::from(payer),
        AccountType::Basic,
        contract_address(),
        AccountType::Oracle,
        Coin::from_u64_unchecked(100),
        data.serialize_to_vec(),
        1,
        NETWORK_ID,
    );
    tx.recipient_data =
        IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, sign(owner, &tx))
            .unwrap();
    tx.proof = sign(payer, &tx).serialize_to_vec();
    tx
}

#[test]
fn it_can_create_oracle_update_transactions() {
    let owner = key_pair(OWNER_PRIVATE_KEY);
    let payer = key_pair(PAYER_PRIVATE_KEY);
    let hashes = vec![keccak_hash(b"state 1"), keccak_hash(b"state 2")];

    let tx = TransactionBuilder::new_update_oracle(
        &payer,
        &owner,
        contract_address(),
        hashes.clone(),
        Coin::from_u64_unchecked(100),
        1,
        NETWORK_ID,
    );

    let expected = make_oracle_signaling_transaction(
        IncomingOracleTransactionData::Update {
            hashes: hashes.clone(),
            proof: SignatureProof::default(),
        },
        &owner,
        &payer,
    );
    assert_eq!(tx, expected);

    assert!(tx.flags.contains(TransactionFlags::SIGNALING));
    assert!(tx.value.is_zero());
    assert_eq!(tx.verify(NETWORK_ID, PROTOCOL_VERSION), Ok(()));

    // The data is signed by the owner, the transaction by the payer.
    match IncomingOracleTransactionData::parse(&tx).unwrap() {
        IncomingOracleTransactionData::Update {
            hashes: tx_hashes,
            proof,
        } => {
            assert_eq!(tx_hashes, hashes);
            assert!(proof.is_signed_by(&Address::from(&owner)));
        }
        data => panic!("Unexpected oracle data: {data:?}"),
    }
    let proof = SignatureProof::deserialize_all(&tx.proof).unwrap();
    assert!(proof.is_signed_by(&Address::from(&payer)));
}

#[test]
fn it_can_create_oracle_change_owner_transactions() {
    let owner = key_pair(OWNER_PRIVATE_KEY);
    let payer = key_pair(PAYER_PRIVATE_KEY);

    let tx = TransactionBuilder::new_change_oracle_owner(
        &payer,
        &owner,
        contract_address(),
        recipient_address(),
        Coin::from_u64_unchecked(100),
        1,
        NETWORK_ID,
    );

    let expected = make_oracle_signaling_transaction(
        IncomingOracleTransactionData::ChangeOwner {
            new_owner: recipient_address(),
            proof: SignatureProof::default(),
        },
        &owner,
        &payer,
    );
    assert_eq!(tx, expected);
    assert_eq!(tx.verify(NETWORK_ID, PROTOCOL_VERSION), Ok(()));
}

#[test]
fn oracle_signaling_transactions_are_rejected_when_data_is_altered() {
    let owner = key_pair(OWNER_PRIVATE_KEY);
    let payer = key_pair(PAYER_PRIVATE_KEY);

    let mut tx = TransactionBuilder::new_update_oracle(
        &payer,
        &owner,
        contract_address(),
        vec![keccak_hash(b"state")],
        Coin::from_u64_unchecked(100),
        1,
        NETWORK_ID,
    );

    // Swapping the hashes after signing must invalidate the owner signature. The payer re-signs,
    // so the owner signature is the only thing left to reject the transaction.
    let mut data = IncomingOracleTransactionData::parse(&tx).unwrap();
    if let IncomingOracleTransactionData::Update { hashes, .. } = &mut data {
        *hashes = vec![keccak_hash(b"forged")];
    }
    tx.recipient_data = data.serialize_to_vec();
    tx.proof = sign(&payer, &tx).serialize_to_vec();
    assert_eq!(
        tx.verify(NETWORK_ID, PROTOCOL_VERSION),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn it_can_create_oracle_delete_transactions() {
    let owner = key_pair(OWNER_PRIVATE_KEY);
    let value = Coin::from_u64_unchecked(1_000);

    let tx = TransactionBuilder::new_delete_oracle(
        &owner,
        contract_address(),
        recipient_address(),
        value,
        Coin::ZERO,
        1,
        NETWORK_ID,
    )
    .unwrap();

    let mut expected = Transaction::new_extended(
        contract_address(),
        AccountType::Oracle,
        vec![],
        recipient_address(),
        AccountType::Basic,
        vec![],
        value,
        Coin::ZERO,
        1,
        NETWORK_ID,
    );
    expected.proof = sign(&owner, &expected).serialize_to_vec();
    assert_eq!(tx, expected);
    assert_eq!(tx.verify(NETWORK_ID, PROTOCOL_VERSION), Ok(()));

    let result = TransactionBuilder::new_delete_oracle(
        &owner,
        contract_address(),
        recipient_address(),
        Coin::ZERO,
        Coin::ZERO,
        1,
        NETWORK_ID,
    );
    assert!(matches!(result, Err(TransactionBuilderError::InvalidValue)));
}

#[test]
fn it_rejects_oracle_creation_with_zero_hash_count() {
    let owner = key_pair(OWNER_PRIVATE_KEY);

    let result = TransactionBuilder::new_create_oracle(
        &owner,
        Address::from(&owner),
        0,
        Coin::from_u64_unchecked(1_000),
        Coin::ZERO,
        1,
        NETWORK_ID,
    );
    assert!(matches!(
        result,
        Err(TransactionBuilderError::InvalidOracleCreation(_))
    ));
}

#[test]
fn it_can_create_bridge_deposit_transactions() {
    let payer = key_pair(PAYER_PRIVATE_KEY);
    let data = hex::decode("0102030405").unwrap();
    let value = Coin::from_u64_unchecked(5_000);

    let tx = TransactionBuilder::new_bridge_deposit(
        &payer,
        contract_address(),
        data.clone(),
        value,
        Coin::from_u64_unchecked(100),
        1,
        NETWORK_ID,
    )
    .unwrap();

    let mut expected = Transaction::new_extended(
        Address::from(&payer),
        AccountType::Basic,
        vec![],
        contract_address(),
        AccountType::Bridge,
        data,
        value,
        Coin::from_u64_unchecked(100),
        1,
        NETWORK_ID,
    );
    expected.proof = sign(&payer, &expected).serialize_to_vec();
    assert_eq!(tx, expected);
    assert_eq!(tx.verify(NETWORK_ID, PROTOCOL_VERSION), Ok(()));

    let result = TransactionBuilder::new_bridge_deposit(
        &payer,
        contract_address(),
        vec![],
        Coin::ZERO,
        Coin::ZERO,
        1,
        NETWORK_ID,
    );
    assert!(matches!(result, Err(TransactionBuilderError::InvalidValue)));
}

fn burn_proof() -> OutgoingTransaction {
    OutgoingTransaction {
        burn_transaction_data: b"burn transaction".to_vec(),
        merkle_proof: AnyMerkleProof::Blake2b(MerkleProof::<Blake2bHash>::new(&[], &[])),
        oracle_state_index: 7,
    }
}

#[test]
fn it_can_create_bridge_release_transactions() {
    let signer = key_pair(PAYER_PRIVATE_KEY);
    let value = Coin::from_u64_unchecked(5_000);
    let fee = Coin::from_u64_unchecked(100);

    let tx = TransactionBuilder::new_bridge_release(
        &signer,
        contract_address(),
        recipient_address(),
        burn_proof(),
        value,
        fee,
        1,
        NETWORK_ID,
    )
    .unwrap();

    // The signature covers the content with a default proof in `sender_data`.
    let mut expected = Transaction::new_extended(
        contract_address(),
        AccountType::Bridge,
        OutgoingBridgeTransactionData {
            burn_proof: burn_proof(),
            proof: SignatureProof::default(),
        }
        .serialize_to_vec(),
        recipient_address(),
        AccountType::Basic,
        vec![],
        value,
        fee,
        1,
        NETWORK_ID,
    );
    let proof = sign(&signer, &expected);
    expected.sender_data =
        OutgoingBridgeTransactionData::set_signature_on_data(&expected.sender_data, proof.clone())
            .unwrap();
    expected.proof = proof.serialize_to_vec();
    assert_eq!(tx, expected);
    assert_eq!(tx.verify(NETWORK_ID, PROTOCOL_VERSION), Ok(()));

    // The embedded proof determines who pays the fee, so it must be the signer's.
    let data = OutgoingBridgeTransactionData::parse(&tx).unwrap();
    assert!(data.proof.is_signed_by(&Address::from(&signer)));
    assert_eq!(data.burn_proof.oracle_state_index, 7);
    assert!(tx.related_addresses().contains(&Address::from(&signer)));

    let result = TransactionBuilder::new_bridge_release(
        &signer,
        contract_address(),
        recipient_address(),
        burn_proof(),
        Coin::ZERO,
        fee,
        1,
        NETWORK_ID,
    );
    assert!(matches!(result, Err(TransactionBuilderError::InvalidValue)));
}
