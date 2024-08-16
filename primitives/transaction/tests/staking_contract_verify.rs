use std::convert::TryInto;

use nimiq_bls::{
    CompressedPublicKey as BlsPublicKey, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey,
};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, Ed25519PublicKey, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::AccountType, coin::Coin, networks::NetworkId, policy::Policy,
    transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_transaction::{
    account::{
        staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
        AccountTransactionVerification,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::key_rng::SecureGenerate;

const VALIDATOR_ADDRESS: &str = "83fa05dbe31f85e719f4c4fd67ebdba2e444d9f8";
const VALIDATOR_PRIVATE_KEY: &str =
    "d0fbb3690f5308f457e245a3cc65ae8d6945155eadcac60d489ffc5583a60b9b";

const VALIDATOR_SIGNING_KEY: &str =
    "b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844";
const VALIDATOR_SIGNING_SECRET_KEY: &str =
    "84c961b11b52a8244ffc5e9d0965bc2dfa6764970f8e4989d45901de401baf27";

const VALIDATOR_VOTING_KEY: &str = "713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a9201";
const VALIDATOR_VOTING_SECRET_KEY: &str =
    "65100f4aa301ded3d9868c3d76052dd0dfede426b51af371dcd8a4a076f11651c86286d2891063ce7b78217a6e163f38ebfde7eb9dcbf5927b2278b00d77329141d44f070620dd6b995455a6cdfe8eee03f657ff255cfb8fb3460ce1135701";

const STAKER_ADDRESS: &str = "8c551fabc6e6e00c609c3f0313257ad7e835643c";
const STAKER_PRIVATE_KEY: &str = "62f21a296f00562c43999094587d02c0001676ddbd3f0acf9318efbcad0c8b43";

#[test]
fn it_does_not_support_contract_creation() {
    let data: Vec<u8> = Vec::with_capacity(0);

    let sender = Address::from([3u8; 20]);

    let transaction = Transaction::new_contract_creation(
        sender,
        AccountType::Basic,
        vec![],
        AccountType::Staking,
        data,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::UnitAlbatross,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
}

#[test]
fn create_validator() {
    let mut rng = test_rng(false);
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_key =
        Ed25519PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap())
            .unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let voting_keypair = bls_key_pair(VALIDATOR_VOTING_SECRET_KEY);

    assert_eq!(voting_key.uncompress().unwrap(), voting_keypair.public_key);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key.clone(),
            proof_of_knowledge: voting_keypair
                .sign(&voting_key.serialize_to_vec())
                .compress(),
            reward_address: Address::from([3u8; 20]),
            signal_data: Some(Blake2bHash::default()),
            proof: SignatureProof::default(),
        },
        Policy::VALIDATOR_DEPOSIT,
        &cold_keypair,
        None,
    );

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103b40400b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a92010303030303030303030303030303030303030303010000000000000000000000000000000000000000000000000000000000000000b7561c15e53da2c482bfafddbf404f28b14ee2743e5cfe451c860da378b2ac23a651b574183d1287e2cea109943a34c44a7df9eb2fe5067c70f1c02bde900828c232a3d7736a278e0e8ac679bc2a1669f660c3810980526b7890f6e1708381007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b530079dbb852cfc6b9571b4bbed6f0f302d8f1deef55640998c2145b56aed007fc1b92222a2778ed3b562f59b23570e6fd1dfb7af07cf08cd6e58f401aa7dba7c70d00000002540be40000000000000000640000000107006200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500614003ac99ddcb92b8af398ff1b554d3b664f033a27b04fe9d265ac426d1fde1b2ea9fbee26bf0c8e62b89f273a984806d79de67e836c9fcec3455639b58480a";
    let tx_size = 731;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Deposit too small or too big.
    tx.value = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT - 100);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    tx.value = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 100);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid proof of knowledge.
    let other_pair = BlsKeyPair::generate(&mut rng);
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
        Policy::VALIDATOR_DEPOSIT,
        &cold_keypair,
        None,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate(&mut rng);

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
        Policy::VALIDATOR_DEPOSIT,
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
    let mut rng = test_rng(false);
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_key =
        Ed25519PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap())
            .unwrap();

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
            new_signal_data: Some(Some(Blake2bHash::default())),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        None,
    );

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103b9040101b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84401713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a92010103030303030303030303030303030303030303030101000000000000000000000000000000000000000000000000000000000000000001b7561c15e53da2c482bfafddbf404f28b14ee2743e5cfe451c860da378b2ac23a651b574183d1287e2cea109943a34c44a7df9eb2fe5067c70f1c02bde900828c232a3d7736a278e0e8ac679bc2a1669f660c3810980526b7890f6e1708381007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300ff327a38d36a5a3aa0052a3c761fd4f820b4289f19522a299004c747676e364522b24317a332bd65f9dcee8a207e7ef5096f7a09c15155a9f3159ca623226306000000000000000000000000000000640000000107026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500da1106e3cc1137a33b7a34101d6a15ad12357280b5d8e1de39702f2890e9f0d597f2e14b2f0e5b45dd6f5fbe9a3e9112b8d3463f1fdd79f77a468386ff916b0a";
    let tx_size = 736;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signaling transaction with a non-zero value.
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
    let other_pair = BlsKeyPair::generate(&mut rng);
    let invalid_pok = other_pair.sign(&voting_key);

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(signing_key),
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
    let other_pair = KeyPair::generate(&mut rng);

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
fn deactivate_validator() {
    let mut rng = test_rng(false);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: VALIDATOR_ADDRESS.parse().unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        None,
    );

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103770283fa05dbe31f85e719f4c4fd67ebdba2e444d9f800b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400ba13522884a716f0688ceeeaabf37e91cbee9798d05ff5f03d3eb2f0ff280f20f83c1327d1f2a908fcecebdfa194283150dde38627c0b91e2c5175c2bef98102000000000000000000000000000000640000000107026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500cc3d26a7c19aca652c22aadb2fee3f104b35dff438d4a4ae3d46f44e762027c3fad96ed24a35460d14be9c5a17eb9b120560ce58a002724ccc2b9dd49c6d510c";
    let tx_size = 285;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signaling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate(&mut rng);

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::DeactivateValidator {
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
    let mut rng = test_rng(false);
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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103770383fa05dbe31f85e719f4c4fd67ebdba2e444d9f800b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844005fe2ead749549fb7d329ef9f99bf5693c87977fd3253400581a8b832d2bef6a705d6fe76ba1e875ade5f0185396cde41622497a9761959adf7ccfca99bcfff07000000000000000000000000000000640000000107026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050000f1a9c7bc1883df69d8dab50661ea56a22b64764b0206fef6f533a47504d924c6574f493f591519ea633e5c7a24fa79f13384d571e5c466c6ca7b0fe8b3100f";
    let tx_size = 285;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signaling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate(&mut rng);

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
fn retire_validator() {
    let mut rng = test_rng(false);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
        None,
    );

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103630400b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400fd7e470f62d7d89158109c0440c2815a6829a962ee838b02fb69840a5b8e6b8b58fe830981328064a45053b28b9abe7ace71838f60c5881b3d352c02f2067908000000000000000000000000000000640000000107026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105003a86c09bd9dc35a226f2444c4c5ce564fda1958571682aaf7548b210d41969c4b2c85b662f5769b9c5916e5abd3b841355fd011ae7b19b7553693621ba76f805";
    let tx_size = 265;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signaling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate(&mut rng);

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::RetireValidator {
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
    let mut rng = test_rng(false);
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(VALIDATOR_ADDRESS.parse().unwrap()),
            proof: SignatureProof::default(),
        },
        Policy::MINIMUM_STAKE,
        &keypair,
        None,
    );

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000000010378050183fa05dbe31f85e719f4c4fd67ebdba2e444d9f800b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050078848bb51c29e0cfa5ef76c851f65366dc788e9f9b8c1cba28972eb9aaa4a2464bb43262985ef0703568865f765460a83e8694b25665f0bc249de3096464df0b000000000098968000000000000000640000000107006200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050032e29c914094d23a4ce65921041227208820b24551b97e72703c8bcc8f3084ba4680e9e8fd926462f021c4a54dce5f864ba2112ba032afcb68a82935cc402503";
    let tx_size = 286;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Deposit too small.
    tx.value = Coin::from_u64_unchecked(Policy::MINIMUM_STAKE - 1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate(&mut rng);

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::CreateStaker {
            delegation: None,
            proof: SignatureProof::default(),
        },
        Policy::VALIDATOR_DEPOSIT,
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
        IncomingStakingTransactionData::AddStake {
            staker_address: STAKER_ADDRESS.parse().unwrap(),
        },
        100,
        &keypair,
        None,
    );

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000000010315068c551fabc6e6e00c609c3f0313257ad7e835643c000000000000006400000000000000640000000107006200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500f5d801f531117483118108b30cd606301a424ba63147f3f0a2e085bd655fd15c8eafb971a39883fc9da3711d7ac474cd6047eed7791ec6e00c6ed1a464fddb01";
    let tx_size = 187;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
}

#[test]
fn update_staker() {
    let mut rng = test_rng(false);
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Test serialization and deserialization.
    let mut tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(VALIDATOR_ADDRESS.parse().unwrap()),
            reactivate_all_stake: false,
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000000010379070183fa05dbe31f85e719f4c4fd67ebdba2e444d9f80000b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050086b3e9305b69d7228967569082a4b870c9dea0ea5e83c4e1f2866f492bc2c974ecc92ea595e23825b3ea7846a85af397c38c32689c3e38c5de244845a2f68b0b000000000000000000000000000000640000000107026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500aad587075d132f1b28335360fa5fed46c9fafd56119606981891cf7f9eec58dcdaebb55bb52a253e9c0cc5e14bb83977b99f4a15f15f48ecce8399796c618606";
    let tx_size = 287;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Signaling transaction with a non-zero value.
    tx.value = Coin::from_u64_unchecked(1);

    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidValue)
    );

    // Invalid signature.
    let other_pair = KeyPair::generate(&mut rng);

    let tx = make_signed_incoming_tx(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            reactivate_all_stake: false,
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
    let tx = make_delete_validator_tx(Policy::VALIDATOR_DEPOSIT - 100, false);

    let tx_hex = "0100000000000000000000000000000000000000010301008c551fabc6e6e00c609c3f0313257ad7e835643c000000000002540be39c000000000000006400000001070062007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b53001450cfe0a974b19c7564411fa9d075d28170a76a8f9f40158be8fa86bfd391e389a7f49e243680947006569dff74dcc09721dd861b8977cf66343a4c6c9fab06";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case (This assumes the delete_validator_tx function creates a tx with 100 fee)
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // This transaction is no longer statically checked for the validator deposit, so the only case where the verification
    // would fail, is by sending a wrong signature
    let tx = make_delete_validator_tx(Policy::VALIDATOR_DEPOSIT - 200, false);

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    let tx = make_delete_validator_tx(Policy::VALIDATOR_DEPOSIT, false);

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong signature.
    let tx = make_delete_validator_tx(Policy::VALIDATOR_DEPOSIT - 100, true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn remove_stake() {
    // Test serialization and deserialization.
    let tx = make_remove_stake_tx(false);

    let tx_hex = "0100000000000000000000000000000000000000010301018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000003e8000000000000006400000001070062007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300cdb476051adbdb069a880d475aefc327f91e760abd588317bc801b2b17292e26197bfb980afcd3d3351d21a48c5e6edd00a215cff9d4bd9b881099b4749ee100";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Transaction::deserialize_from_vec(&hex::decode(tx_hex).unwrap()).unwrap();
    assert_eq!(tx, deser_tx);

    // Works in the valid case.
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // Wrong signature.
    let tx = make_remove_stake_tx(true);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
fn outgoing_transaction_data_is_below_max_transaction_sender() {
    assert!(Policy::MAX_TX_SENDER_DATA_SIZE >= OutgoingStakingTransactionData::MAX_SIZE);
}

#[test]
fn incoming_transaction_data_is_below_max_transaction_sender() {
    assert!(Policy::MAX_TX_RECIPIENT_DATA_SIZE >= IncomingStakingTransactionData::MAX_SIZE);
}

fn make_incoming_tx(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateValidator { .. }
        | IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::AddStake { .. } => Transaction::new_extended(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            vec![],
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            data.serialize_to_vec(),
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            1,
            NetworkId::UnitAlbatross,
        ),
        _ => Transaction::new_signaling(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::UnitAlbatross,
        ),
    }
}

fn make_signed_incoming_tx(
    data: IncomingStakingTransactionData,
    value: u64,
    in_key_pair: &KeyPair,
    wrong_pk: Option<Ed25519PublicKey>,
) -> Transaction {
    let mut tx = make_incoming_tx(data, value);

    let in_proof = SignatureProof::from_ed25519(
        match wrong_pk {
            None => in_key_pair.public,
            Some(pk) => pk,
        },
        in_key_pair.sign(&tx.serialize_content()),
    );

    tx.recipient_data =
        IncomingStakingTransactionData::set_signature_on_data(&tx.recipient_data, in_proof)
            .unwrap();

    let out_private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let out_key_pair = KeyPair::from(out_private_key);

    let out_proof = SignatureProof::from_ed25519(
        out_key_pair.public,
        out_key_pair.sign(&tx.serialize_content()),
    )
    .serialize_to_vec();

    tx.proof = out_proof;

    tx
}

fn make_delete_validator_tx(value: u64, wrong_sig: bool) -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        OutgoingStakingTransactionData::DeleteValidator.serialize_to_vec(),
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        vec![],
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let wrong_pk = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
    .public;

    let sig = SignatureProof::from_ed25519(
        match wrong_sig {
            false => key_pair.public,
            true => wrong_pk,
        },
        key_pair.sign(&tx.serialize_content()),
    );

    tx.proof = sig.serialize_to_vec();

    tx
}

fn make_remove_stake_tx(wrong_sig: bool) -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        OutgoingStakingTransactionData::RemoveStake.serialize_to_vec(),
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        vec![],
        1000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let wrong_pk = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
    .public;

    let sig = SignatureProof::from_ed25519(
        match wrong_sig {
            false => key_pair.public,
            true => wrong_pk,
        },
        key_pair.sign(&tx.serialize_content()),
    );

    tx.proof = sig.serialize_to_vec();

    tx
}

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}
