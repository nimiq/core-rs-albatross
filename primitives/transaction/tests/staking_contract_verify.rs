use std::convert::TryInto;

use nimiq_bls::{
    CompressedPublicKey as BlsPublicKey, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey,
};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, EdDSAPublicKey, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::AccountType, coin::Coin, networks::NetworkId, policy::Policy,
    transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_transaction::{
    account::{
        staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
        AccountTransactionVerification,
    },
    EdDSASignatureProof, SignatureProof, Transaction,
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
        EdDSAPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103940400b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a9201030303030303030303030303030303030303030300b7561c15e53da2c482bfafddbf404f28b14ee2743e5cfe451c860da378b2ac23a651b574183d1287e2cea109943a34c44a7df9eb2fe5067c70f1c02bde900828c232a3d7736a278e0e8ac679bc2a1669f660c3810980526b7890f6e1708381007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b53002461492d55e961dfc52e88d74653002781abc47f5f869c96ecb63d0a362745ea5e7479cd5d8c136e9f9aaa81ad0395a933c683b57d161ccbba0c04989685070c000000003b9aca0000000000000000640000000104006200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500b70fa1a22db6ccfbd58845f038f04049edf9050ca0fc3664a5c196dca0df5dc7cd0f3f10718b6f7faad0352b8d668da356ee17f37ee198ae2588519ddaa3eb0c";
    let tx_size = 699;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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
        EdDSAPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103b9040101b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84401713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a92010103030303030303030303030303030303030303030101000000000000000000000000000000000000000000000000000000000000000001b7561c15e53da2c482bfafddbf404f28b14ee2743e5cfe451c860da378b2ac23a651b574183d1287e2cea109943a34c44a7df9eb2fe5067c70f1c02bde900828c232a3d7736a278e0e8ac679bc2a1669f660c3810980526b7890f6e1708381007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300295608af6c7274e006e73c5feb3c9d2702f10495a1b1bdbfb511e27620b68ba45510c1fa0cfae02f30340977f7fe02f182e735510ce4c0bb1bd71c94eec8750a000000000000000000000000000000640000000104026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd9105004ff4444f57727aa2bd7ca971d587d2fd21ff2a1545f99e6a77d2e2482c6417abcddef22173c16537dc437a2091c42cb335dea90a02e39262e5abda2b02b7f70c";
    let tx_size = 736;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103770283fa05dbe31f85e719f4c4fd67ebdba2e444d9f800b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400e281586c62ba61c1139a95dd1ab02454ed0c2d4eaf0441eca4f21baefa9b37dc6ad3f64aa9e93e42e0349f931799504c14c19e6282bb77f9bea7620b1eb0f106000000000000000000000000000000640000000104026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050010fbe4b82ce0f02a32fc7c90a00c8ec20904041423bd3239ec355cb8589cf1b173ed8999063d9e1f7afe72aa2a2c2fe83ac38fd63adecb3929e4cdd85d4d6505";
    let tx_size = 285;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103770383fa05dbe31f85e719f4c4fd67ebdba2e444d9f800b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400deaba5374c18fe4c11c9ed8e9d7c9dd62b177689675b123074a943880a33941e73c5d9b1ecccd8fab51bee29337f0672cb34fd01fc7a18830f03bfd10880ad00000000000000000000000000000000640000000104026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500d2c62b32d87cd77d70232c7cc0935dd5bdf093ec8db07b7081063d7a8a514a7b9d43fad4dca7cc1b1a9a8dd32011afb1fc84f3716ddac8f819cd84130a60ce04";
    let tx_size = 285;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c0000000000000000000000000000000000000000000103630400b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a84400214acc4c13b8563a1b03ca59ebc18fde38a668e3250d72d7d826d6c6398da9f5558e4a1f9ad179e17c3c82203c4691b8500be81d9bd150b60e8b1dc6c434870d000000000000000000000000000000640000000104026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500ce063800488dc13de4fc76ed056988ef667a2a9bb1d1a216695a05bde87fb38d1ce2acae2276e8969571a4576ddf3880f0b98b46ec5d32110f894d8833725906";
    let tx_size = 265;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000000010378050183fa05dbe31f85e719f4c4fd67ebdba2e444d9f800b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500cc5c3405654f4d05aab84e25bc093f89493580eb815de6d8b96649a80c2b96a277e51194f66a353d21b42a5c276a183f9602306862cf2a79e4730402af82d601000000000000006400000000000000640000000104006200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd91050022567a37f8d530949120430ac001ba5ff5728561e46140b701fddd7f6f3f4d960ab422bdedc2adc1376645ac06f35667a78f2bde99f146b558640d2ea715b30c";
    let tx_size = 286;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000000010315068c551fabc6e6e00c609c3f0313257ad7e835643c000000000000006400000000000000640000000104006200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500ec9d59330a36169dc602dbdab7c4ec1a289b9a9b97938219db0585da337195b651def72d25b8a29c7b00936ac44a21f59db467c8e0646cfe9641b88e306f650d";
    let tx_size = 187;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000000000000000000000000000000010379070183fa05dbe31f85e719f4c4fd67ebdba2e444d9f80000b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500e1c9a7664f978647e563434d6b4be25127bdd509d483577613a8f47a9dc231a18389f57b976eedf5905d31bed6cd74b1432c51faed9956a13ade677d50d1620a000000000000000000000000000000640000000104026200b3adb13fe6887f6cdcb8c82c429f718fcdbbb27b2a19df7c1ea9814f19cd910500e39ce80d5183929fd737d2f916ce17f81beae41423c33ca70e1a35f94cece4798a24e22a548583dad2ea7dffa28d3110206adcae51f3b70345df1e25f8cb5005";
    let tx_size = 287;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "0100000000000000000000000000000000000000010301008c551fabc6e6e00c609c3f0313257ad7e835643c0000000000003b9ac99c000000000000006400000001040062007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300f4469ca005b396f7ef274aa872dc585d7a6ce33177ef5ea2c9208056a8cb16431c5756d62b58288b34c73966322dbf8555d9e346d8e545e4b34d273b5cb3240a";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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

    let tx_hex = "0100000000000000000000000000000000000000010301018c551fabc6e6e00c609c3f0313257ad7e835643c000000000000000003e8000000000000006400000001040062007451b039e2f3fcafc3be7c6bd9e01fbc072c956a2b95a335cfb3cd3702335b5300008cddeff67b3d9703b5d5ec1a6fe5165b27135fa7f14151fb43dd9c4948a76528c417ec13871779df77d4373d237c04d09b705962b812817d5f97d8109cdf0a";
    let tx_size = 167;

    let mut ser_tx: Vec<u8> = Vec::with_capacity(tx_size);
    assert_eq!(tx_size, tx.serialized_size());
    assert_eq!(tx_size, tx.serialize_to_writer(&mut ser_tx).unwrap());
    assert_eq!(tx_hex, hex::encode(ser_tx));

    let deser_tx = Deserialize::deserialize_from_vec(&hex::decode(tx_hex).unwrap()[..]).unwrap();
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
            vec![],
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            data.serialize_to_vec(),
            value.try_into().unwrap(),
            100.try_into().unwrap(),
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
    wrong_pk: Option<EdDSAPublicKey>,
) -> Transaction {
    let mut tx = make_incoming_tx(data, value);

    let in_proof = SignatureProof::EdDSA(EdDSASignatureProof::from(
        match wrong_pk {
            None => in_key_pair.public,
            Some(pk) => pk,
        },
        in_key_pair.sign(&tx.serialize_content()),
    ));

    tx.recipient_data =
        IncomingStakingTransactionData::set_signature_on_data(&tx.recipient_data, in_proof)
            .unwrap();

    let out_private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let out_key_pair = KeyPair::from(out_private_key);

    let out_proof = SignatureProof::EdDSA(EdDSASignatureProof::from(
        out_key_pair.public,
        out_key_pair.sign(&tx.serialize_content()),
    ))
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
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let wrong_pk = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
    .public;

    let sig = SignatureProof::EdDSA(EdDSASignatureProof::from(
        match wrong_sig {
            false => key_pair.public,
            true => wrong_pk,
        },
        key_pair.sign(&tx.serialize_content()),
    ));

    tx.proof = sig.serialize_to_vec();

    tx
}

fn make_unstake_tx(wrong_sig: bool) -> Transaction {
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
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let wrong_pk = KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
    .public;

    let sig = SignatureProof::EdDSA(EdDSASignatureProof::from(
        match wrong_sig {
            false => key_pair.public,
            true => wrong_pk,
        },
        key_pair.sign(&tx.serialize_content()),
    ));

    tx.proof = sig.serialize_to_vec();

    tx
}

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}
