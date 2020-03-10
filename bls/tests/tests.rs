use std::vec::Vec;

use algebra::bls12_377::{Fq, Fr, G1Affine, G1Projective};
use algebra_core::fields::PrimeField;
use algebra_core::UniformRand;

use num_traits::identities::One;
use rand::SeedableRng;
use rand_xorshift::XorShiftRng;

use nimiq_bls::*;
use nimiq_hash::{Blake2sHasher, Hasher};
use nimiq_utils::key_rng::Rng;

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

        assert_eq!(
            keypair.public_key.compress().uncompress().unwrap(),
            keypair.public_key
        );
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

#[test]
fn hash_to_g1_encoding() {
    for i in 0..100 {
        let message = format!("Message {}", i);
        let hash = Blake2sHasher::new().chain(&message).finish();
        let point = test_hash_to_g1_encoding(hash.clone());
        let point2 = Signature::hash_to_g1(hash);
        assert_eq!(point, point2);
    }
}

pub fn bytes_to_bits(bytes: &[u8]) -> Vec<bool> {
    let mut bits = vec![];
    for i in 0..bytes.len() {
        let byte = bytes[i];
        for j in (0..8).rev() {
            bits.push((byte >> j) & 1 == 1);
        }
    }

    bits
}

fn test_hash_to_g1_encoding(h: nimiq_hash::Blake2sHash) -> G1Projective {
    // This extends the input hash from 32 bytes to 48 bytes using the Blake2X algorithm.
    // See https://blake2.net/blake2x.pdf for more details.
    let mut bytes = vec![];
    let digest_length = vec![32, 16];
    for i in 0..2 {
        let blake2x = crypto_primitives::prf::blake2s::Blake2sWithParameterBlock {
            digest_length: digest_length[i],
            key_length: 0,
            fan_out: 0,
            depth: 0,
            leaf_length: 32,
            node_offset: i as u32,
            xof_digest_length: 48,
            node_depth: 0,
            inner_length: 32,
            salt: [0; 8],
            personalization: [0; 8],
        };
        let mut state = blake2_rfc::blake2s::Blake2s::with_parameter_block(&blake2x.parameters());
        state.update(h.as_bytes());
        let mut result = state.finalize().as_bytes().to_vec();
        bytes.append(&mut result);
    }

    // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve point. At this time, it is not guaranteed to be a valid point.
    let mut bits = bytes_to_bits(&bytes);
    let y_coordinate = bits[0];
    // Set highest relevant bit to false.
    bits[7] = false;
    let x_coordinate = Fq::from_repr(algebra::BigInteger::from_bits(&bits[7..384]));

    // y-coordinate is at first bit.
    let y_coordinate2 = (bytes[0] >> 7) & 1 == 1;
    bytes[0] = 0; // Set highest 7 padding bits to 0, plus the 8th bit so that it is guaranteed to be smaller than the modulus.
    let mut x_coordinate2 = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[..]));
    assert_eq!(x_coordinate, x_coordinate2);
    assert_eq!(y_coordinate, y_coordinate2);

    // This implements the try-and-increment method of converting an integer to an elliptic curve point.
    // See https://eprint.iacr.org/2009/226.pdf for more details.
    loop {
        let point = G1Affine::get_point_from_x(x_coordinate2, y_coordinate2);
        if point.is_some() {
            let point = G1Affine::from(point.unwrap());
            let g1 = point.scale_by_cofactor();
            return g1;
        }
        x_coordinate2 += &Fq::one();
    }
}
