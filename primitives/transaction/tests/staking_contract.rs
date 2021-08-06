use std::convert::TryInto;

use beserial::{Deserialize, Serialize};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_bls::SecretKey as BlsSecretKey;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy::{STAKING_CONTRACT_ADDRESS, VALIDATOR_DEPOSIT};
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::{SignatureProof, Transaction, TransactionError};
use nimiq_utils::key_rng::SecureGenerate;

const VALIDATOR_ADDRESS: &str = "83fa05dbe31f85e719f4c4fd67ebdba2e444d9f8";
const VALIDATOR_PRIVATE_KEY: &str =
    "d0fbb3690f5308f457e245a3cc65ae8d6945155eadcac60d489ffc5583a60b9b";

const VALIDATOR_WARM_KEY: &str = "7182b1c2d0e2377d69413dc14c56cd923b67e41e";
const VALIDATOR_WARM_SECRET_KEY: &str =
    "84c961b11b52a8244ffc5e9d0965bc2dfa6764970f8e4989d45901de401baf27";

const VALIDATOR_HOT_KEY: &str = "003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61";
const VALIDATOR_HOT_SECRET_KEY: &str =
    "b552baff2c2cc4937ec3531c833c3ffc08f92a95b3ba4a53cf7e8c99ef9db99b99559b8dbb8f3c44fa5671da42cc2633759aea71c1b696ea18df5451d0d43a225a882b29a1091ece16e82f664c2c6f2b360c7b6ce84e5d0995ae45290dbd0000";

const STAKER_ADDRESS: &str = "8c551fabc6e6e00c609c3f0313257ad7e835643c";
const STAKER_PRIVATE_KEY: &str = "62f21a296f00562c43999094587d02c0001676ddbd3f0acf9318efbcad0c8b43";

// The following code is kept as a reference on how to generate the constants.
// #[test]
// fn gen_ed25519_keypair() {
//     let pair = KeyPair::generate_default_csprng();
//     println!("Address: {}", Address::from(&pair.public).to_hex());
//     println!("Private Key: {}", pair.private.to_hex());
//     assert!(false)
// }
//
// #[test]
// fn gen_bls_keypair() {
//     let pair = BlsKeyPair::generate_default_csprng();
//     println!("Public Key: {}", pair.public_key.to_string());
//     println!("Secret Key: {}", pair.secret_key.to_string());
//     assert!(false)
// }

#[test]
fn it_does_not_support_contract_creation() {
    let data: Vec<u8> = Vec::with_capacity(0);

    let sender = Address::from([3u8; 20]);

    let transaction = Transaction::new_contract_creation(
        data,
        sender,
        AccountType::Basic,
        AccountType::Staking,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
}

#[test]
fn create_validator_transaction_verifies() {
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let warm_address =
        Address::deserialize_from_vec(&hex::decode(VALIDATOR_WARM_KEY).unwrap()).unwrap();

    let hot_pk =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_HOT_KEY).unwrap()).unwrap();

    let hot_keypair = bls_key_pair(VALIDATOR_HOT_SECRET_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            warm_key: warm_address.clone(),
            validator_key: hot_pk.clone(),
            proof_of_knowledge: hot_keypair.sign(&hot_pk.serialize_to_vec()).compress(),
            reward_address: Address::from([3u8; 20]),
            signal_data: None,
            proof: SignatureProof::default(),
        },
        VALIDATOR_DEPOSIT,
        &cold_keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Deposit too small or too big.
    tx.value = Coin::from_u64_unchecked(VALIDATOR_DEPOSIT - 100);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    tx.value = Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 100);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid proof of knowledge.
    let other_pair = BlsKeyPair::generate_default_csprng();
    let invalid_pok = other_pair.sign(&hot_pk);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            warm_key: warm_address.clone(),
            validator_key: hot_pk.clone(),
            proof_of_knowledge: invalid_pok.compress(),
            reward_address: Address::from([3u8; 20]),
            signal_data: None,
            proof: SignatureProof::default(),
        },
        VALIDATOR_DEPOSIT,
        &cold_keypair,
        None,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            warm_key: warm_address,
            validator_key: hot_pk.clone(),
            proof_of_knowledge: hot_keypair.sign(&hot_pk.serialize_to_vec()).compress(),
            reward_address: Address::from([3u8; 20]),
            signal_data: None,
            proof: SignatureProof::default(),
        },
        VALIDATOR_DEPOSIT,
        &cold_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn update_validator_transaction_verifies() {
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let warm_address =
        Address::deserialize_from_vec(&hex::decode(VALIDATOR_WARM_KEY).unwrap()).unwrap();

    let hot_pk =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_HOT_KEY).unwrap()).unwrap();

    let hot_keypair = bls_key_pair(VALIDATOR_HOT_SECRET_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_warm_key: Some(warm_address.clone()),
            new_validator_key: Some(hot_pk.clone()),
            new_proof_of_knowledge: Some(hot_keypair.sign(&hot_pk.serialize_to_vec()).compress()),
            new_reward_address: Some(Address::from([3u8; 20])),
            new_signal_data: Some(Some(Blake2bHash::default())),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Doing no updates.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_warm_key: None,
            new_validator_key: None,
            new_proof_of_knowledge: None,
            new_reward_address: None,
            new_signal_data: None,
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        None,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Invalid proof of knowledge.
    let other_pair = BlsKeyPair::generate_default_csprng();
    let invalid_pok = other_pair.sign(&hot_pk);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_warm_key: Some(warm_address.clone()),
            new_validator_key: Some(hot_pk.clone()),
            new_proof_of_knowledge: Some(invalid_pok.compress()),
            new_reward_address: Some(Address::from([3u8; 20])),
            new_signal_data: Some(Some(Blake2bHash::default())),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        None,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_warm_key: Some(warm_address),
            new_validator_key: Some(hot_pk.clone()),
            new_proof_of_knowledge: Some(hot_keypair.sign(&hot_pk.serialize_to_vec()).compress()),
            new_reward_address: Some(Address::from([3u8; 20])),
            new_signal_data: Some(Some(Blake2bHash::default())),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn retire_validator_transaction_verifies() {
    let warm_keypair = ed25519_key_pair(VALIDATOR_WARM_SECRET_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &warm_keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &warm_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn reactivate_validator_transaction_verifies() {
    let warm_keypair = ed25519_key_pair(VALIDATOR_WARM_SECRET_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &warm_keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &warm_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn unpark_validator_transaction_verifies() {
    let warm_keypair = ed25519_key_pair(VALIDATOR_WARM_SECRET_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &warm_keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &warm_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn create_staker_transaction_verifies() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(VALIDATOR_ADDRESS.parse().unwrap()),
            proof: SignatureProof::default(),
        },
        100,
        &keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Deposit too small.
    tx.value = Coin::ZERO;

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::ZeroValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: None,
            proof: SignatureProof::default(),
        },
        VALIDATOR_DEPOSIT,
        &keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn stake_transaction_verifies() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            staker_address: STAKER_ADDRESS.parse().unwrap(),
        },
        100,
        &keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
}

#[test]
fn update_staker_transaction_verifies() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(VALIDATOR_ADDRESS.parse().unwrap()),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn retire_staker_transaction_verifies() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireStaker {
            value: Coin::from_u64_unchecked(10),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Retiring zero stake.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireStaker {
            value: Coin::ZERO,
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireStaker {
            value: Coin::from_u64_unchecked(10),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn reactivate_staker_transaction_verifies() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Works in the valid case.
    let mut tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateStaker {
            value: Coin::from_u64_unchecked(10),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Retiring zero stake.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateStaker {
            value: Coin::ZERO,
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateStaker {
            value: Coin::from_u64_unchecked(10),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn drop_validator_transaction_verifies() {
    // Works in the valid case.
    let tx = make_drop_validator_transaction(VALIDATOR_DEPOSIT - 100, false);

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong values.
    let tx = make_drop_validator_transaction(VALIDATOR_DEPOSIT - 200, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    let tx = make_drop_validator_transaction(VALIDATOR_DEPOSIT, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Wrong signature.
    let tx = make_drop_validator_transaction(VALIDATOR_DEPOSIT - 100, true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn unstake_transaction_verifies() {
    // Works in the valid case.
    let tx = make_unstake_transaction(false);

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong signature.
    let tx = make_unstake_transaction(true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn deduct_fees_transaction_verifies() {
    // Works in the valid case.
    let tx = make_deduct_fees_transaction(0, false);

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong value.
    let tx = make_deduct_fees_transaction(1, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Wrong signature.
    let tx = make_deduct_fees_transaction(0, true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateValidator { .. }
        | IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::Stake { .. } => Transaction::new_extended(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
        _ => Transaction::new_signalling(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
    }
}

fn make_signed_incoming_transaction(
    data: IncomingStakingTransactionData,
    value: u64,
    in_key_pair: &KeyPair,
    wrong_pk: Option<PublicKey>,
) -> Transaction {
    let mut tx = make_incoming_transaction(data, value);

    let in_proof = SignatureProof::from(
        match wrong_pk {
            None => in_key_pair.public,
            Some(pk) => pk,
        },
        in_key_pair.sign(&tx.serialize_content()),
    );

    tx.data = IncomingStakingTransactionData::set_signature_on_data(&tx.data, in_proof).unwrap();

    let out_private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let out_key_pair = KeyPair::from(out_private_key);

    let out_proof = SignatureProof::from(
        out_key_pair.public,
        out_key_pair.sign(&tx.serialize_content()),
    )
    .serialize_to_vec();

    tx.proof = out_proof;

    tx
}

fn make_drop_validator_transaction(value: u64, wrong_sig: bool) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
        AccountType::Staking,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let wrong_pk = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
    .public;

    let sig = SignatureProof::from(
        match wrong_sig {
            false => key_pair.public,
            true => wrong_pk,
        },
        key_pair.sign(&tx.serialize_content()),
    );

    let proof = OutgoingStakingTransactionProof::DropValidator { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn make_unstake_transaction(wrong_sig: bool) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
        AccountType::Staking,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        1000.try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let wrong_pk = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
    .public;

    let sig = SignatureProof::from(
        match wrong_sig {
            false => key_pair.public,
            true => wrong_pk,
        },
        key_pair.sign(&tx.serialize_content()),
    );

    let proof = OutgoingStakingTransactionProof::Unstake { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn make_deduct_fees_transaction(value: u64, wrong_sig: bool) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
        AccountType::Staking,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let wrong_pk = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
    .public;

    let sig = SignatureProof::from(
        match wrong_sig {
            false => key_pair.public,
            true => wrong_pk,
        },
        key_pair.sign(&tx.serialize_content()),
    );

    let proof = OutgoingStakingTransactionProof::DeductFees {
        from_active_balance: true,
        proof: sig,
    };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}
