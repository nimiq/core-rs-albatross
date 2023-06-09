use std::convert::TryInto;

use nimiq_bls::{
    CompressedPublicKey as BlsPublicKey, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey,
};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey};
use nimiq_primitives::{
    account::AccountType, coin::Coin, networks::NetworkId, policy::Policy,
    transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_transaction::{
    account::{
        staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionProof},
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
    let mut rng = test_rng(false);
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

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
            signal_data: None,
            proof: SignatureProof::default(),
        },
        Policy::VALIDATOR_DEPOSIT,
        &cold_keypair,
        None,
    );

    let tx_hex = "01950400b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a9201030303030303030303030303030303030303030300b7561c15e53da2c482bfafddbf404f28b14ee2743e5cfe451c860da378b2ac23a651b574183d1287e2cea109943a34c44a7df9eb2fe5067c70f1c02bde900828c232a3d7736a278e0e8ac679bc2a1669f660c3810980526b7890f6e17083817451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b53000000a9a1bf05b23a1b0580afd69a8640ee34810d0b4badc63c57a9869a8cb92c5b03fed3e6b8e44c938ef4584829610d9c72ae6c82bfa3b2254c537c8e0e5e5e830b8c551fabc6e6e00c609c3f0313257ad7e835643c00000000000000000000000000000000000000000103000000003b9aca00000000000000006400000001040063b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000db8dff9fb9f36cef3ae42663b60e5f6d0ee33d93077093d7617557391f10ca8d1e8dfb82b187757cb1636667ecbe1e78c55ce1cd52955b40a1b7b45623a7f30c";
    let tx_size = 700;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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
            new_signal_data: Some(Some(Blake2bHash::default())),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
        None,
    );

    let tx_hex = "01ba040101b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84401713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a92010103030303030303030303030303030303030303030101000000000000000000000000000000000000000000000000000000000000000001b7561c15e53da2c482bfafddbf404f28b14ee2743e5cfe451c860da378b2ac23a651b574183d1287e2cea109943a34c44a7df9eb2fe5067c70f1c02bde900828c232a3d7736a278e0e8ac679bc2a1669f660c3810980526b7890f6e17083817451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300000033949d8cf59d9cc1ca63731051672a17b7228f059710af637099e3600de1b2aac52706ef5570cc8625fc047605732d919f59e6a3e03c6e8acde8469a08ad8f0a8c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000001030000000000000000000000000000006400000001040263b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050000003f4f5273a31f514d01e0e4ff730227b65017d9aabdfa45fb6c69de743b70c189db09759d3b214746b79fba4d170863fa547e9b80594256076a9d082ac24d700a";
    let tx_size = 737;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "01780283fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a8440000007702068ea6e94fd43e2e4ea10939f425c57d390b913499178bcc18482fa4e18be470ae2ab758928f5d4ad7e2683866eab57a4ef6c2dc147bef8966f43ed7b0098c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000001030000000000000000000000000000006400000001040263b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000b360871f71cf8df07b7b19840717d57ffed0e27f07ca4ada6c57a1aa19eb6c989e9fb370f8a2084deabe8f6b0657adb9ffa3fdc75e3e9efb04d1fad8a033c502";
    let tx_size = 286;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "01780383fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a8440000009b7c37f603363656fc52b8215df91db2453d5196af687d2befbf09125febdeaf3dafc62ab5bbc0fc4a5a8cac99a726b54924b756a6acd43bfdf81f16ac4125058c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000001030000000000000000000000000000006400000001040263b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000363fac6d9d4adabdfa62d2b39bb52fcb4c879ff46f4b42fd1e3e35899aa928b321588e15e8360bdbbf87c6636967a2099285f3a450b042d9f244fdc8153cd407";
    let tx_size = 286;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "016404b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a8440000008b627d0985714a1e0940da28e1b2a00d1f02fa9cf8536e9c944fda48102b8c698afc707e40f270456ab75902105cf9a60eeb3877d70a01da76cce77c3000b7008c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000001030000000000000000000000000000006400000001040263b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000a3c3cc099c6ead7b0a17f0e5fbbf53b1777b0f05c49bd6122e67074f0045225d9d3e798ee483afadc7b94f97919302e7d7b9473eb9123aa12223480ea232ff06";
    let tx_size = 266;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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
        100,
        &keypair,
        None,
    );

    let tx_hex = "0179050183fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000354538daf6bd78fd32f213447d7920cbedf263c9dc54b1659a6fc8e08e0c4f0b28c0f18c8af5d9dd65ab7fc8a5e3bc20dca23aded182dedb60ce4e3ae65b490a8c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000001030000000000000064000000000000006400000001040063b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050000008e14cc60f3502c508a20d3f5b554466874485d5e204a0d06b2c5e7da995a6939cd020c7db39008b4fb513b7277ec963d9a288064c7b015f0ed2d753538e28308";
    let tx_size = 287;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "0115068c551fabc6e6e00c609c3f0313257ad7e835643c8c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000001030000000000000064000000000000006400000001040063b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000a887927b90af2469bff456751f5850bdb2fdf3321e2892ac88f2def812eb15fe0ca5571a5badfa9732f53e33b8397c2ff45427020681ae4967044bd8aac46200";
    let tx_size = 187;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
        None,
    );

    let tx_hex = "0179070183fa05dbe31f85e719f4c4fd67ebdba2e444d9f8b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000b5ba97691f51803d57a58a35ef7a755d045a611d835abcab02e2960ccf0f40b639869282da13610655af32fbd2316c916821c4582c6c448cb96ee706cb2eae088c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000001030000000000000000000000000000006400000001040263b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105000000ffb3e60d86b516f5aef19d8196c9183cf2403555b5d30a9bb51b0c3df9a43b1a1ff2e8d53805bb2b3218d7fc13368f9b5d66e000e5a521cd2f0e218fc1698901";
    let tx_size = 287;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "01000000000000000000000000000000000000000001038c551fabc6e6e00c609c3f0313257ad7e835643c00000000003b9ac99c000000000000006400000001040064007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b53000000f8bf2e05747d3938d7b1cde2632765527e95e2d5a683de900de9cbaac5b91ddb5c8060033f6159725f8be3ab463c174ad36425e81dc4c4f13848e9828cec1d04";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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
fn unstake() {
    // Test serialization and deserialization.
    let tx = make_unstake_tx(false);

    let tx_hex = "01000000000000000000000000000000000000000001038c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000003e8000000000000006400000001040064017451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b530000004117db88880fdf5886f1ea6cf160790852ddf54f7effedacc6d617cc91141c4657e4eb1ef9c3cf4cb0b9bcb4e235f21b8bb565d1ba28e996d83f0b1de4f9d600";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx =
        Deserialize::deserialize_from_vec(&mut &hex::decode(tx_hex).unwrap()[..]).unwrap();
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
        | IncomingStakingTransactionData::AddStake { .. } => Transaction::new_extended(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
        _ => Transaction::new_signaling(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
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
        Policy::STAKING_CONTRACT_ADDRESS,
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
        Policy::STAKING_CONTRACT_ADDRESS,
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

    let proof = OutgoingStakingTransactionProof::RemoveStake { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}
