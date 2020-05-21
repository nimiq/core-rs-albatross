use rand::thread_rng;

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
