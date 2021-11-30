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

const VALIDATOR_SIGNING_KEY: &str =
    "b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844";
const VALIDATOR_SIGNING_SECRET_KEY: &str =
    "84c961b11b52a8244ffc5e9d0965bc2dfa6764970f8e4989d45901de401baf27";

const VALIDATOR_VOTING_KEY: &str = "003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61";
const VALIDATOR_VOTING_SECRET_KEY: &str =
    "b552baff2c2cc4937ec3531c833c3ffc08f92a95b3ba4a53cf7e8c99ef9db99b99559b8dbb8f3c44fa5671da42cc2633759aea71c1b696ea18df5451d0d43a225a882b29a1091ece16e82f664c2c6f2b360c7b6ce84e5d0995ae45290dbd0000";

const STAKER_ADDRESS: &str = "8c551fabc6e6e00c609c3f0313257ad7e835643c";
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
            signing_key: signing_key.clone(),
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

    let tx_hex = "01021300b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce6103030303030303030303030303030303030303030080a5bbe3d7b6ddf63149071b2c35c4bd930bf9c38999a165dc8cc54ca989fd2b373cf83d685edcc158be7831f0cc0e697d2ec5636cec431a6956e9480637240c0bc2e9601c5da967f3785a07ca97785c9c954c81dbe1ef630bdd15facbff697451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b53004fd5865b202fb1e50aaee150c6a8c50fc7486b855ddf9d93dfae02334529686ac4d57e603cabbfaacb3c7d5665cfc6c9b1e93a800993fd7b52a65ea4565b50048c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000003b9aca0000000000000000640000000104000061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500e4a8aa7519bd0bae69ee07877b38fa9cde52763569e421259e72dd4126c257de4b37f65c35f4ee17216817f9abf66b9915a009da5ee5bfa683ede2c70c9c1a03";
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
            signing_key: signing_key.clone(),
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
            new_signing_key: Some(signing_key.clone()),
            new_voting_key: Some(voting_key.clone()),
            new_proof_of_knowledge: Some(
                voting_keypair
                    .sign(&voting_key.serialize_to_vec())
                    .compress(),
            ),
            new_reward_address: Some(Address::from([3u8; 20])),
            new_signal_data: Some(Some(Blake2bHash::default())),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        None,
    );

    let tx_hex = "0102380101b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84401003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61010303030303030303030303030303030303030303010100000000000000000000000000000000000000000000000000000000000000000180a5bbe3d7b6ddf63149071b2c35c4bd930bf9c38999a165dc8cc54ca989fd2b373cf83d685edcc158be7831f0cc0e697d2ec5636cec431a6956e9480637240c0bc2e9601c5da967f3785a07ca97785c9c954c81dbe1ef630bdd15facbff697451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b530029430a47e14ff8e6df3b29fa2a53a704a03a1aa53195dccd3f812fc7a13438941c836f0f710ec75f638351bdf735abf8283a74e812c606df8dcf185f9f3df20f8c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105001fa0fa933290b4b6a1861789ae718463d65c3c509d4feafbfab3f6f9054302cb7f9e88a7bbc930667f0347012e0fd5a66843f874ec1cff76865bf382d2c4a005";
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
            new_signing_key: Some(signing_key.clone()),
            new_voting_key: Some(voting_key.clone()),
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
fn retire_validator() {
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::RetireValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        None,
    );

    let tx_hex = "0100760283fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400a2e66f4cf5db793d328de015de77cd46896a627671332acc43effdd118e4db685079ce26d4f7671dd4d15dda8405764ca190b991caedbd6335570d4852983d038c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050030e654e691993ecf42f4ab130698d00a0c252d66840e3362eed5b68cee4f5bfb26f6bdfaab909624fe7a389f838c285248e035d927557f8d01e22fc159408b03";
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
        IncomingStakingTransactionData::RetireValidator {
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

    let tx_hex = "0100760383fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844009fa6e1312d2309d65b4c953687887a3f1fa795d24813ae7f73cb15d3dc2f995a567326d875696d15b38043d251260fd3e6af16b6be487590da0b0b12596210098c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050098ff55ae9d04752b718dcb1347e33074a99c55701a6d1c2e2f1643115a9f71aa0f5359e88562bf5287978d031c947d3bd819f518e3611ea0cc28ce295f7dc501";
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

    let tx_hex = "0100760483fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844002ae6dc307bbf6254d8df6e231e9e92ccc9b2206177c253f66af6a15b319f49d80363142a0afedd1dcc782fca9788a3ba6dc46c54656f51fbcda9e866681203058c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500cd4aabe977fb63752419c7567b00b21fae11eb9b298561a86c8a6e8840c185c5a3c0779bb68797e898d4304bb239299d4bad186d33939a0e9a2280823f93d70d";
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

    let tx_hex = "010077050183fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105001c883e79523866f32f9d434847e77911bf358d7382f12506eb672a653b41822e16695aa9007c4e574c4c929e86fb2a54de4ebf4e269939104446dadc9b3ebe098c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000006400000000000000640000000104000061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105002240c1fe4bc584effc4ec1f581941c168cf60a230c80294521faa1c63b64c4eb4d9279494163b657b9a6ccaa3f6879f63dfb5d433e52bd21e7b184f9fbf09209";
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

    let tx_hex = "010015068c551fabc6e6e00c609c3f0313257ad7e835643c8c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000006400000000000000640000000104000061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500689cdfa3e5d6b039ded4e886ebf9b98140db4fb89e80812ce62f836a481dcc66429100fd27b5946fd7e5659a35c581b1deef7280d60dd0adf90cf7a6819a6409";
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

    let tx_hex = "010077070183fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105008c6e8d9843fb9daaf756bd75d6a476ede114f58b6b6f920a7d35aee3afefb93e9f3ac6c6d2c694676a33344aeb1a5d215f62ad88e1c6ab3415ba061ae7ee4f038c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050034aedc06cd84fded28e4b95170fc3215f9b544718d6965adfbe66df08950f2e02e21b51603525178bfd82669701f030abde6d4884aee6f50dea5367209acee00";
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
fn retire_staker() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::RetireStaker {
            value: Coin::from_u64_unchecked(10),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    let tx_hex = "01006a08000000000000000ab3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500e1750e6e97c2f314c15aec215d0c6003dd412b99ed0f845c925771e9c54864b82db562fa1231b3d7d1658b4b6f1485a3b3fb74dfd7cb8b71bbda869a0b8aa80c8c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105001030aac58d1749d5b336edc9d85e35cee4e94dd5b5aa88d879bcaded7120af8e4c8aacead0ec90c382790dfa46ab38711e53fe35912830dfe8975871cb2fbb0a";
    let tx_size = 272;

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

    // Retiring zero stake.
    let tx = make_signed_incoming_tx(
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

    let tx = make_signed_incoming_tx(
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
fn reactivate_staker() {
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::ReactivateStaker {
            value: Coin::from_u64_unchecked(10),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    let tx_hex = "01006a09000000000000000ab3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105001df828abac1cebcf399189764bd37d06d9e691b7ba87a58363076898bd2b26aec1e79532eef2e233224ec424381b7df3b36f9ba4477cf89a52e4af37c93fd5088c551fabc6e6e00c609c3f0313257ad7e835643c00d6d530da000000000000602dbca99b000000000003000000000000000000000000000000640000000104020061b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105007601e2f7359d8be6cf0ac726d02be594a8407072462379eb1f2fcdc51b416ee09b07fa97f3dd6ae510df1e0ecc052aca6e8b6fc0be6355aeddc04b576f788c02";
    let tx_size = 272;

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

    // Retiring zero stake.
    let tx = make_signed_incoming_tx(
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

    let tx = make_signed_incoming_tx(
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
fn drop_validator() {
    // Test serialization and deserialization.
    let tx = make_drop_validator_tx(VALIDATOR_DEPOSIT - 100, false);

    let tx_hex = "010000d6d530da000000000000602dbca99b0000000000038c551fabc6e6e00c609c3f0313257ad7e835643c00000000003b9ac99c00000000000000640000000104000062007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300f59b9c7c01cd154de300f0106650b7f3a7d79af892ddb69c96f4010e9f5fe78160a371e8d801da9f55e079687474601898857d02168f160d47041dade476280a";
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
    let tx = make_drop_validator_tx(VALIDATOR_DEPOSIT - 200, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    let tx = make_drop_validator_tx(VALIDATOR_DEPOSIT, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Wrong signature.
    let tx = make_drop_validator_tx(VALIDATOR_DEPOSIT - 100, true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn unstake() {
    // Test serialization and deserialization.
    let tx = make_unstake_tx(false);

    let tx_hex = "010000d6d530da000000000000602dbca99b0000000000038c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000003e800000000000000640000000104000062017451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300d7fd73bd8ceff5a16c6140b406df865803be7903ef3f4d2006621b6d262c8de3951c78fbfa26f8bbea8074f9c356cb66f81ea3b7e77e95f2d535653280176504";
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

#[test]
fn deduct_fees() {
    // Test serialization and deserialization.
    let tx = make_deduct_fees_tx(0, false);

    let tx_hex = "010000d6d530da000000000000602dbca99b0000000000038c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000064000000010400006302017451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b530039e052ac741ab14e1aab1b8c9e0ebd289ed5c8b70271f82c0cb0421a5e7ec6a8c030f37c7db0193f76c755843b7bcb3e735a69875c833893640b4b7cf5b8240d";
    let tx_size = 168;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong value.
    let tx = make_deduct_fees_tx(1, false);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Wrong signature.
    let tx = make_deduct_fees_tx(0, true);

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

fn make_drop_validator_tx(value: u64, wrong_sig: bool) -> Transaction {
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

    let proof = OutgoingStakingTransactionProof::DropValidator { proof: sig };

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

fn make_deduct_fees_tx(value: u64, wrong_sig: bool) -> Transaction {
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
