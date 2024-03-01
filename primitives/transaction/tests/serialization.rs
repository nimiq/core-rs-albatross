use std::convert::{TryFrom, TryInto};

use nimiq_keys::{Address, ES256PublicKey, ES256Signature, PublicKey, Signature};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId};
use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use nimiq_test_log::test;
use nimiq_transaction::*;

const EXTENDED_TRANSACTION: &str = "014a88aaad038f9b8248865c4b9249efc554960e160000ad25610feb43d75307763d3f010822a757027429000000000746a52880000000000000000000000136c32a00e2010e4712ea5b1703873529dd195b2b8f014c295ab352a12e3332d8f30cfc2db9680480c77af04feb0d89bdb5d5d9432d4ca17866abf3b4d6c1a05fa0fbdaed056181eaff68db063c759a0964bceb5f262f7335ed97c5471e773429926c106eae50881b998c516581e6d93933bb92feb2edcdbdb1b118fc000f8f1df8715538840b79e74721c631efe0f9977ccd88773b022a07b3935f2e8546e20ed7f7e1a0c77da7a7e1737bf0625170610846792ea16bc0f6d8cf9ded8a9da1d467f4191a3a97d5fc17d08d699dfa486787f70eb09e2cdbd5b63fd1a8357e1cd24cd37aa2f3408400";
const BASIC_TRANSACTION: &str = "00000222666efadc937148a6d61589ce6d4aeecca97fda4c32348d294eab582f14a0754d1260f15bea0e8fb07ab18f45301483599e34000000000000c350000000000000008a00019640023fecb82d3aef4be76853d5c5b263754b7d495d9838f6ae5df60cf3addd3512a82988db0056059c7a52ae15285983ef0db8229ae446c004559147686d28f0a30a";
const INVALID_EXTENDED_TRANSACTION: &str = "014a88aaad038f9b8248865c4b9249efc554960e16000000ad25610feb43d75307763d3f010822a75702742900000000000746a52880000000000000000000000136c32a0500e20e4712ea5b1703873529dd195b2b8f014c295ab352a12e3332d8f30cfc2db9680480c77af04feb0d89bdb5d5d9432d4ca17866abf3b4d6c1a05fa0fbdaed056181eaff68db063c759a0964bceb5f262f7335ed97c5471e773429926c106eae50881b998c516581e6d93933bb92feb2edcdbdb1b118fc000f8f1df8715538840b79e74721c631efe0f9977ccd88773b022a07b3935f2e8546e20ed7f7e1a0c77da7a7e1737bf0625170610846792ea16bc0f6d8cf9ded8a9da1d467f4191a3a97d5fc17d08d699dfa486787f70eb09e2cdbd5b63fd1a8357e1cd24cd37aa2f3408400";

#[test]
fn it_can_deserialize_historic_transaction() {
    let v: Vec<u8> = hex::decode(EXTENDED_TRANSACTION).unwrap();
    let t: Transaction = Deserialize::deserialize_from_vec(&v[..]).unwrap();
    assert_eq!(t.recipient_data, Vec::<u8>::new());
    assert_eq!(
        t.sender,
        Address::from(&hex::decode("4a88aaad038f9b8248865c4b9249efc554960e16").unwrap()[..])
    );
    assert_eq!(t.sender_type, AccountType::Basic);
    assert_eq!(
        t.recipient,
        Address::from(&hex::decode("ad25610feb43d75307763d3f010822a757027429").unwrap()[..])
    );
    assert_eq!(t.recipient_type, AccountType::Basic);
    assert_eq!(t.value, Coin::try_from(8000000000000u64).unwrap());
    assert_eq!(t.fee, Coin::ZERO);
    assert_eq!(t.validity_start_height, 79555);
    assert_eq!(t.network_id, NetworkId::Main);
    assert_eq!(t.flags, TransactionFlags::empty());
    assert_eq!(t.proof, hex::decode("0e4712ea5b1703873529dd195b2b8f014c295ab352a12e3332d8f30cfc2db9680480c77af04feb0d89bdb5d5d9432d4ca17866abf3b4d6c1a05fa0fbdaed056181eaff68db063c759a0964bceb5f262f7335ed97c5471e773429926c106eae50881b998c516581e6d93933bb92feb2edcdbdb1b118fc000f8f1df8715538840b79e74721c631efe0f9977ccd88773b022a07b3935f2e8546e20ed7f7e1a0c77da7a7e1737bf0625170610846792ea16bc0f6d8cf9ded8a9da1d467f4191a3a97d5fc17d08d699dfa486787f70eb09e2cdbd5b63fd1a8357e1cd24cd37aa2f3408400").unwrap())
}

#[test]
fn deserialize_fails_on_invalid_transaction_flags() {
    let v: Vec<u8> = hex::decode(INVALID_EXTENDED_TRANSACTION).unwrap();
    let t: Result<Transaction, DeserializeError> = Deserialize::deserialize_from_vec(&v[..]);
    assert_eq!(t, Err(DeserializeError::serde_custom()));
}

#[test]
fn it_can_serialize_historic_transaction() {
    let v: Vec<u8> = hex::decode(EXTENDED_TRANSACTION).unwrap();
    let t: Transaction = Deserialize::deserialize_from_vec(&v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(t.serialized_size());
    let size = t.serialize_to_writer(&mut v2).unwrap();
    assert_eq!(size, t.serialized_size());
    assert_eq!(hex::encode(v2), EXTENDED_TRANSACTION);
}

#[test]
fn it_can_deserialize_basic_transaction() {
    let v: Vec<u8> = hex::decode(BASIC_TRANSACTION).unwrap();
    let t: Transaction = Deserialize::deserialize_from_vec(&v[..]).unwrap();
    assert_eq!(t.recipient_data, Vec::<u8>::new());
    assert_eq!(
        t.sender,
        Address::from(&hex::decode("b02b9d9fcfa1a60dabe65165ded66a26983404dc").unwrap()[..])
    );
    assert_eq!(t.sender_type, AccountType::Basic);
    assert_eq!(
        t.recipient,
        Address::from(&hex::decode("754d1260f15bea0e8fb07ab18f45301483599e34").unwrap()[..])
    );
    assert_eq!(t.recipient_type, AccountType::Basic);
    assert_eq!(t.value, 50000u64.try_into().unwrap());
    assert_eq!(t.fee, 138u64.try_into().unwrap());
    assert_eq!(t.validity_start_height, 104000);
    assert_eq!(t.network_id, NetworkId::Dev);
    assert_eq!(t.flags, TransactionFlags::empty());
    assert_eq!(t.proof, hex::decode("000222666efadc937148a6d61589ce6d4aeecca97fda4c32348d294eab582f14a0003fecb82d3aef4be76853d5c5b263754b7d495d9838f6ae5df60cf3addd3512a82988db0056059c7a52ae15285983ef0db8229ae446c004559147686d28f0a30a").unwrap())
}

#[test]
fn it_can_serialize_basic_transaction() {
    let v: Vec<u8> = hex::decode(BASIC_TRANSACTION).unwrap();
    let t: Transaction = Deserialize::deserialize_from_vec(&v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(t.serialized_size());
    let size = t.serialize_to_writer(&mut v2).unwrap();
    assert_eq!(size, t.serialized_size());
    assert_eq!(hex::encode(v2), BASIC_TRANSACTION);
}

#[test]
fn it_can_serialize_and_deserialize_signature_proofs() {
    let proof = SignatureProof::try_from_webauthn(
        PublicKey::ES256(
            ES256PublicKey::from_bytes(
                &hex::decode("02915782665472928bfe72c2869bbbd6bc0c239379d5a150ea5e2b19b205d53659").unwrap()
            )
            .unwrap(),
        ),
        None,
        Signature::ES256(
            ES256Signature::from_bytes(&hex::decode("07b917e958f6fafcad747ac95e20ddf1ac63fc5d99bf4516e902e94591641084015ef7ed46034af18512743a0dcbc7a786aae27110b8cbd1cce81b062bd80c6e").unwrap())
            .unwrap(),
        ),
        &hex::decode("49960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d97630165019a6c").unwrap(),
        br#"{"type":"webauthn.get","challenge":"4rk3LpNhR-jlyPRHP-xgniidFviD-pbL1hSyh5Nole8","origin":"http://localhost:3000","crossOrigin":false}"#,
    ).unwrap();

    let serialized = Serialize::serialize_to_vec(&proof.clone());

    let deserialized: SignatureProof = Deserialize::deserialize_from_vec(&serialized[..]).unwrap();

    assert_eq!(deserialized.public_key, proof.public_key);
    assert_eq!(deserialized.merkle_path, proof.merkle_path);
    assert_eq!(deserialized.signature, proof.signature);
    assert_eq!(deserialized.webauthn_fields, proof.webauthn_fields);
}
