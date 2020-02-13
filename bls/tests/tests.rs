use nimiq_bls::*;

use algebra::{fields::bls12_377::Fr, rand::UniformRand};
use nimiq_utils::key_rng::Rng;
use rand::SeedableRng;
use rand_xorshift::XorShiftRng;
use std::vec::Vec;

// Fast, but insecure, keypair generation. Meant only for testing.
#[cfg(test)]
fn generate_predictable<R: Rng>(rng: &mut R) -> KeyPair {
    let secret = SecretKey {
        secret_key: Fr::rand(rng),
    };
    return KeyPair::from_secret(&secret);
}

#[test]
fn sign_verify() {
    let mut rng = XorShiftRng::from_seed([
        0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86, 0x55,
        0x54,
    ]);

    for i in 0..100 {
        let keypair = generate_predictable(&mut rng);
        let message = format!("Message {}", i);
        let sig = keypair.sign(&message);
        assert_eq!(keypair.verify(&message.as_bytes(), &sig), true);
    }
}

#[test]
fn compress_uncompress() {
    let mut rng = XorShiftRng::from_seed([
        0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86, 0x55,
        0x54,
    ]);

    for i in 0..100 {
        let keypair = generate_predictable(&mut rng);
        let message = format!("Message {}", i);
        let sig = keypair.sign(&message);

        assert_eq!(keypair.public_key.compress().uncompress().unwrap(), keypair.public_key);
        assert_eq!(sig.compress().uncompress().unwrap(), sig);
    }
}

#[test]
fn aggregate_signatures_same_messages() {
    let mut rng = XorShiftRng::from_seed([
        0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86, 0x55,
        0x54,
    ]);

    let mut public_keys = Vec::with_capacity(1000);
    let message = "Same message";
    let mut signatures = Vec::with_capacity(1000);
    for _ in 0..100 {
        let keypair = generate_predictable(&mut rng);
        let signature = keypair.sign(&message);
        public_keys.push(keypair.public_key);
        signatures.push(signature);
    }

    let akey = AggregatePublicKey::from_public_keys(&public_keys);
    let asig = AggregateSignature::from_signatures(&signatures);

    assert_eq!(akey.verify(&message, &asig), true);
}
