use ark_ec::ProjectiveCurve;
use rand::thread_rng;

use beserial::{Deserialize, Serialize};
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

#[test]
fn aggregate_signatures_serialization() {
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
    let ser_agg_sig = agg_sig.serialize_to_vec();

    assert_eq!(
        AggregateSignature::deserialize_from_vec(&ser_agg_sig).unwrap(),
        agg_sig
    );

    assert!(agg_key.verify(
        &message,
        &AggregateSignature::deserialize_from_vec(&ser_agg_sig).unwrap(),
    ));
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
    let hex_public_key = "806e5e55c41ad0787220850095170fba6921bb1b9b5fef3a35feb96ce9b367984db2bc0f9c025d6d5c5cd62ba796f4c2bf7b086c1f44dfda30efe2a56ec2ebfa22d36a70499b7cd910cf79b6ef69179bf149715ec1163a6911e8f4e9aa26e8";
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
fn serialize_deserialize() {
    let rng = &mut thread_rng();

    for i in 0..100 {
        let keypair = KeyPair::generate(rng);
        let ser_pub_key = keypair.public_key.serialize_to_vec();
        let compress_pub_key = keypair.public_key.compress();
        let ser_comp_pub_key = compress_pub_key.serialize_to_vec();
        let message = format!("Message {}", i);

        let sig = keypair.sign(&message);
        let ser_signature = sig.serialize_to_vec();
        let ser_comp_signature = sig.compress().serialize_to_vec();

        // Check that we can deserialize a serialized public key
        assert_eq!(
            PublicKey::deserialize_from_vec(&ser_pub_key).unwrap(),
            keypair.public_key
        );

        // Check that we can deserialize a serialized compressed public key
        assert_eq!(
            CompressedPublicKey::deserialize_from_vec(&ser_comp_pub_key)
                .unwrap()
                .uncompress()
                .unwrap(),
            keypair.public_key
        );

        // Check that we can deserialize a serialized signature
        assert_eq!(
            Signature::deserialize_from_vec(&ser_signature).unwrap(),
            sig
        );

        assert_eq!(
            Signature::deserialize_from_vec(&ser_signature)
                .unwrap()
                .compressed,
            sig.compressed
        );

        // Check that we can deserialize a serialized compressed signature
        assert_eq!(
            CompressedSignature::deserialize_from_vec(&ser_comp_signature)
                .unwrap()
                .uncompress()
                .unwrap(),
            sig
        );

        assert_eq!(
            sig.compressed,
            CompressedSignature::deserialize_from_vec(&ser_comp_signature)
                .unwrap()
                .uncompress()
                .unwrap()
                .compressed
        );
    }
}
