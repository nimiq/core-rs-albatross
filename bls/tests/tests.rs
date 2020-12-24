use ark_ec::ProjectiveCurve;
use rand::thread_rng;

use beserial::Deserialize;
use nimiq_bls::*;
use nimiq_utils::key_rng::SecureGenerate;

// Warning: You really should run these tests on release mode. Otherwise it will take too long.

#[test]
fn sign_verify() {
    let rng = &mut thread_rng();

    for i in 0..100 {
        let keypair = KeyPair::generate(rng);

        let message = format!("Message {}", i);

        let sig = keypair.sign(&message);

        assert!(keypair.verify(&message, &sig));
    }
}

#[test]
fn compress_uncompress() {
    let rng = &mut thread_rng();

    for i in 0..100 {
        let keypair = KeyPair::generate(rng);

        let message = format!("Message {}", i);

        let sig = keypair.sign(&message);

        assert_eq!(
            keypair.public_key.compress().uncompress().unwrap(),
            keypair.public_key
        );

        assert_eq!(sig.compress().uncompress().unwrap(), sig);
    }
}

#[test]
fn uncompress_compress() {
    let hex_public_key = "01535b85d472b233642cce4f5ffd3b32e3dbd518a0124614a91cc6628d0d77a7e9d955125548c56b6c7812daa41519aaf8a2d9dbfb84f4b30ac6d18ee2619a015a1097fa25bd885bbc31ae4fb961884e4cf941cecdd25a70e6a0a726ba4b2d01696d325876808c592716569672d403fb41f19bc50e18e3df855bf6f053484de4be63658875dff127681681c9574d1d0c5d048053ec1b291234145f46167de7628bbaf971d8d89e8c6c29b5e2bc47cbd3be65331194822096b4cf092f644e004b7a2fc2cbeebc88d375095e2913127ca2de9eae486fbb0a8a671ff517a81169066ea1dca6e6745498f9ad5586b4c74ba5de7cbbe39ed4ec10714ca253d5f4fcc379f0a06a762b83e676bec4e6835899d6e639f4c90a00f1d3852f239b71";
    let raw_public_key: Vec<u8> = hex::decode(hex_public_key).unwrap();
    let compressed_public_key: CompressedPublicKey =
        Deserialize::deserialize_from_vec(&raw_public_key).unwrap();

    println!(
        "{:?}",
        compressed_public_key
            .uncompress()
            .unwrap()
            .public_key
            .into_affine()
    );

    assert_eq!(
        compressed_public_key.uncompress().unwrap().compress(),
        compressed_public_key
    );
}

#[test]
fn aggregate_signatures_same_message() {
    let rng = &mut thread_rng();

    let message = "Same message";

    let mut public_keys = Vec::new();

    let mut signatures = Vec::new();

    for _ in 0..100 {
        let keypair = KeyPair::generate(rng);

        let signature = keypair.sign(&message);

        public_keys.push(keypair.public_key);

        signatures.push(signature);
    }

    let agg_key = AggregatePublicKey::from_public_keys(&public_keys);

    let agg_sig = AggregateSignature::from_signatures(&signatures);

    assert!(agg_key.verify(&message, &agg_sig));
}
