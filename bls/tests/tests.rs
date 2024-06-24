use ark_ec::CurveGroup;
use nimiq_bls::*;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_utils::key_rng::SecureGenerate;

// Warning: You really should run these tests on release mode. Otherwise it will take too long.

#[test]
fn sign_verify() {
    let rng = &mut test_rng(false);

    for i in 0..100 {
        let keypair = KeyPair::generate(rng);

        let message = format!("Message {}", i);

        let sig = keypair.sign(&message);

        assert!(keypair.verify(&message, &sig));
    }
}

#[test]
fn compress_uncompress() {
    let rng = &mut test_rng(false);

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
fn serialize_deserialize() {
    let rng = &mut test_rng(false);

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

#[test]
fn uncompress_compress() {
    let hex_public_key = "ae4cc2e31e04add9a6d379b4379b02f302971503cbac8d02fdc5d2dc8204d24ec8d095627d037de747f1a8ea7bf3c1693262d947f78e0cc73c18ecc2f2ec5b2249d551e1680fe0c973a7951bd78d4fbe0326be71286ed34004d2443eb3b00167a02edffcfd2b8539448fa116c5454da2d181dc03ea8cfe3fedb58b9b945d5e506c794deb3ba73983005b3ff799212bf59030a8dd17ff48fd5d015695195a022fed8ba4fab28a4c3e2d6f41be0e6315e41824df161219c02be5a281c215011c13131184187e9100d2d6a5321fd9b154806ecc78e93b91331a5334b8876fd1b8ea62b17ce6045fc9e1af60b7705b0cf86dba79f5bcb8320c99a45f3b7c7178f8f87ba953de2755c61af882059c1de1d7a35357f06cd4a7d954e4bb211900";
    let raw_public_key: Vec<u8> = hex::decode(hex_public_key).unwrap();
    let compressed_public_key = CompressedPublicKey::deserialize_from_vec(&raw_public_key).unwrap();

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
    let rng = &mut test_rng(false);

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
    let rng = &mut test_rng(false);

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
        &AggregateSignature::deserialize_from_vec(&ser_agg_sig).unwrap()
    ));
}
