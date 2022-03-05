use std::convert::TryInto;

use beserial::{Deserialize, Serialize};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_bls::SecretKey as BlsSecretKey;
use nimiq_hash::Blake3Hash;
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

const VALIDATOR_SIGNING_KEY: &str =
    "b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844";
const VALIDATOR_SIGNING_SECRET_KEY: &str =
    "84c961b11b52a8244ffc5e9d0965bc2dfa6764970f8e4989d45901de401baf27";

const VALIDATOR_VOTING_KEY: &str = "003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61";
const VALIDATOR_VOTING_SECRET_KEY: &str =
    "b552baff2c2cc4937ec3531c833c3ffc08f92a95b3ba4a53cf7e8c99ef9db99b99559b8dbb8f3c44fa5671da42cc2633759aea71c1b696ea18df5451d0d43a225a882b29a1091ece16e82f664c2c6f2b360c7b6ce84e5d0995ae45290dbd0000";

const STAKER_ADDRESS: &str = "24ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f";
const STAKER_PRIVATE_KEY: &str = "62f21a296f00562c43999094587d02c0001676ddbd3f0acf9318efbcad0c8b43";

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
fn create_validator() {
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let voting_keypair = bls_key_pair(VALIDATOR_VOTING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key.clone(),
            proof_of_knowledge: voting_keypair
                .sign(&voting_key.serialize_to_vec())
                .compress(),
            reward_address: Address::from([3u8; 20]),
            signal_data: None,
            proof: SignatureProof::default(),
        },
        VALIDATOR_DEPOSIT,
        &cold_keypair,
        None,
    );

    let tx_hex = "01021300b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce6103030303030303030303030303030303030303030080a5bbe3d7b6ddf63149071b2c35c4bd930bf9c38999a165dc8cc54ca989fd2b373cf83d685edcc158be7831f0cc0e697d2ec5636cec431a6956e9480637240c0bc2e9601c5da967f3785a07ca97785c9c954c81dbe1ef630bdd15facbff697451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300c2dcec1ba9890a34df61949a9fb3a8f57d3aea31fa126d97546460c492d583162c47940f768254f7667f79b86bf988e9db69cf2dbf5826a31c55606c2e60aa0e24ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000003b9aca0000000000000000640000000104000061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105003b39bf346fe8fc02b05286fb53db8cc9536415b1ab4faf7d197568b886c17a66e373888edeaafbff37274bd743cc490265e579a5f4085903200aebad024e5b0b";
    let tx_size = 697;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
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
    let invalid_pok = other_pair.sign(&voting_key);

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key.clone(),
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

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key.clone(),
            proof_of_knowledge: voting_keypair
                .sign(&voting_key.serialize_to_vec())
                .compress(),
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
fn update_validator() {
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let voting_keypair = bls_key_pair(VALIDATOR_VOTING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(signing_key),
            new_voting_key: Some(voting_key.clone()),
            new_proof_of_knowledge: Some(
                voting_keypair
                    .sign(&voting_key.serialize_to_vec())
                    .compress(),
            ),
            new_reward_address: Some(Address::from([3u8; 20])),
            new_signal_data: Some(Some(Blake3Hash::default())),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        None,
    );

    let tx_hex = "0102380101b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84401003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61010303030303030303030303030303030303030303010100000000000000000000000000000000000000000000000000000000000000000180a5bbe3d7b6ddf63149071b2c35c4bd930bf9c38999a165dc8cc54ca989fd2b373cf83d685edcc158be7831f0cc0e697d2ec5636cec431a6956e9480637240c0bc2e9601c5da967f3785a07ca97785c9c954c81dbe1ef630bdd15facbff697451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b530046ccd66b7349406ee39ad5438daf862cae05e026ecb50ea40f69aa7d0b076d49e4236c9344d78709124a9d40816dfa19e7236ee0a3f4cc91594777e8c13a8f0e24ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500f55b5e3d81c187ce7403c281adf209f258023b2fedbba0aa3459fef93b60fc86d9c039d2055e1e853f22016932668256fa6c23f3a6b6cc8cba303f10e4ee950b";
    let tx_size = 734;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Doing no updates.
    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: None,
            new_voting_key: None,
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
    let invalid_pok = other_pair.sign(&voting_key);

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(signing_key),
            new_voting_key: Some(voting_key.clone()),
            new_proof_of_knowledge: Some(invalid_pok.compress()),
            new_reward_address: Some(Address::from([3u8; 20])),
            new_signal_data: Some(Some(Blake3Hash::default())),
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

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(signing_key),
            new_voting_key: Some(voting_key.clone()),
            new_proof_of_knowledge: Some(
                voting_keypair
                    .sign(&voting_key.serialize_to_vec())
                    .compress(),
            ),
            new_reward_address: Some(Address::from([3u8; 20])),
            new_signal_data: Some(Some(Blake3Hash::default())),
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
fn retire_validator() {
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        None,
    );

    let tx_hex = "0100760283fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400eba80b249ec4030f540cc9e2ba08587e49f4cb4546fcaaec62f6fb61420b6e3aaf876635042557e7bd809fb543ee449cd2606c15e8a5ddf82a0e753e0ebe4b0224ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050084777522f932c98cf4e464c6aea1ef334d1942125fe6fc02a6c010322af338b766d29679d0b0dace03eb98b005260d7c5c90c643c9be6377f5ec4a0f9462e901";
    let tx_size = 284;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn reactivate_validator() {
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        None,
    );

    let tx_hex = "0100760383fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400cbb8679e7cd4aeb13cd09768375b2901f0331215c0c26d28e1eda8f849dc9e8a6a0136c9572d6c7fb922d5deb49a4da77890a085e61bd9df0b9579a3512df20224ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500d7aa230d05f79af5b8995af36f907a7f0b7a0d0ad22707f6aefb7463e98220dbc3bda1ade541edb59b10b3f640a054fa2ba5170cbcf6cb8638d0eaa733909307";
    let tx_size = 284;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn unpark_validator() {
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        None,
    );

    let tx_hex = "0100760483fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a8440057b20ca04a03cbdc2b507eaaab3c4ab395f087a516e472f4f94a1fcf7c50e1472a8819903bb059ca31b9d76c50482bf8b400474add3e0424d740d6ad5db7710924ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500c6340a782354e5793f0d1710241c7a9afa9f026c16df8cf1a6ef7dd1f8f7670cc77b1aa77799bdaad53b06675b3ea36a178772b6ab2725da5f7666a9f84d260c";
    let tx_size = 284;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        Some(other_pair.public),
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn create_staker() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(VALIDATOR_ADDRESS.parse().unwrap()),
            proof: SignatureProof::default(),
        },
        100,
        &keypair,
        None,
    );

    let tx_hex = "010077050183fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500ae9b69f1c8bb96d8d35e43f7d8d6328ce98b2eae5afe953d3379d7d415ee78cc9225b20a68ac2e9ecccd9cd7a80c5f352db8f2f41a1b6e1663677b5d016a2c0424ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000000000006400000000000000640000000104000061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050032f03c5bea874248d22115badef90e25e69dfcc30bb2a04eb01b0f76de24fae7e8ccf02dd61409729813b57b67eb47a80260cc5397401d8c544285514636dc02";
    let tx_size = 285;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Deposit too small.
    tx.value = Coin::ZERO;

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::ZeroValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_tx(
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
fn stake() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Test serialization and deserialization.
    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::Stake {
            staker_address: STAKER_ADDRESS.parse().unwrap(),
        },
        100,
        &keypair,
        None,
    );

    let tx_hex = "0100150624ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f24ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000000000006400000000000000640000000104000061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105006d9f55a2188ad26f5d2f2a03a408d8e2a68fc5786add09c8d473f8f4fb8206506130444777ce796ce4ce82051d917c8d756b0ab2227ec05cb0e7831b66231402";
    let tx_size = 187;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
}

#[test]
fn update_staker() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(VALIDATOR_ADDRESS.parse().unwrap()),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    let tx_hex = "010077070183fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500d9c1a169f1733c21a5ea507727161ced5c9235a4132faa6454c93ee2a2fbca0d61069119805cf3c7239bf3cba6ff696efaad19e59c4c8a8b8a66bc9ec46ba80024ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050007bebe92f79f8e92ba6fb1bf435b15cca8e6ad9563b0a52cce04bf843100f8e6d64a68f0a98a852fb12895454288de425242b09920ed165cf94662c72c29560f";
    let tx_size = 285;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signalling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate_default_csprng();

    let tx = make_signed_incoming_tx(
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
fn delete_validator() {
    // Test serialization and deserialization.
    let tx = make_delete_validator_tx(VALIDATOR_DEPOSIT - 100, false);

    let tx_hex = "010000d6d530da000000000000602dbca99b00000000000324ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f00000000003b9ac99c00000000000000640000000104000062007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300e7a0f68b5c885a6aa7ca570bd57b35b220c5c5a477c018342e2fb66bb20a0c34ba5ddd43e753572e6f30f2ec32ce99ea7cce44ce5a4dec7450ddba2a56168207";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong values.
    let tx = make_delete_validator_tx(VALIDATOR_DEPOSIT - 200, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    let tx = make_delete_validator_tx(VALIDATOR_DEPOSIT, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Wrong signature.
    let tx = make_delete_validator_tx(VALIDATOR_DEPOSIT - 100, true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn unstake() {
    // Test serialization and deserialization.
    let tx = make_unstake_tx(false);

    let tx_hex = "010000d6d530da000000000000602dbca99b00000000000324ccdcebcbf3e595f5bf85bfb260c13d1ff9f39f0000000000000003e800000000000000640000000104000062017451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300aa9957699ecafabe557b75340d72b0145a812c9defba3e7682ba2d87ebcb124097fe0d0f47c5590d49de14ed87aea65061819f293d29b20254a3667cb86e0a04";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong signature.
    let tx = make_unstake_tx(true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

fn make_incoming_tx(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateValidator { .. }
        | IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::Stake { .. } => Transaction::new_extended(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            STAKING_CONTRACT_ADDRESS,
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
            STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
    }
}

fn make_signed_incoming_tx(
    data: IncomingStakingTransactionData,
    value: u64,
    in_key_pair: &KeyPair,
    wrong_pk: Option<PublicKey>,
) -> Transaction {
    let mut tx = make_incoming_tx(data, value);

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

fn make_delete_validator_tx(value: u64, wrong_sig: bool) -> Transaction {
    let mut tx = Transaction::new_extended(
        STAKING_CONTRACT_ADDRESS,
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

    let proof = OutgoingStakingTransactionProof::DeleteValidator { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn make_unstake_tx(wrong_sig: bool) -> Transaction {
    let mut tx = Transaction::new_extended(
        STAKING_CONTRACT_ADDRESS,
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

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}
