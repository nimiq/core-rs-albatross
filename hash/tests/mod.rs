use std::io::Write;

use nimiq_hash::{
    argon2kdf,
    sha512::{Sha512Hash, Sha512Hasher},
    Blake2bHash, Blake2bHasher, Blake2sHash, Blake2sHasher, HashOutput, Hasher, Keccak256Hash,
    Keccak256Hasher, Sha256Hash, Sha256Hasher,
};
use nimiq_mmr::hash::Merge;
use nimiq_test_log::test;

mod hmac;
mod pbkdf2;

#[test]
fn it_can_compute_sha256() {
    // sha256('test') = '9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08'

    assert_eq!(
        Sha256Hasher::default().digest(b"test"),
        Sha256Hash::from("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08")
    );
    let mut h = Sha256Hasher::default();
    h.write_all(b"te").unwrap();
    h.write_all(b"st").unwrap();
    assert_eq!(
        h.finish(),
        Sha256Hash::from("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08")
    );
}

#[test]
fn it_can_compute_blake2b() {
    // blake2b('test') = '928b20366943e2afd11ebc0eae2e53a93bf177a4fcf35bcc64d503704e65e202'

    assert_eq!(
        Blake2bHasher::default().digest(b"test"),
        Blake2bHash::from("928b20366943e2afd11ebc0eae2e53a93bf177a4fcf35bcc64d503704e65e202")
    );
    let mut h = Blake2bHasher::default();
    h.write_all(b"te").unwrap();
    h.write_all(b"st").unwrap();
    assert_eq!(
        h.finish(),
        Blake2bHash::from("928b20366943e2afd11ebc0eae2e53a93bf177a4fcf35bcc64d503704e65e202")
    );
}

#[test]
fn it_can_compute_blake2s() {
    // blake2s('test') = 'f308fc02ce9172ad02a7d75800ecfc027109bc67987ea32aba9b8dcc7b10150e'

    assert_eq!(
        Blake2sHasher::default().digest(b"test"),
        Blake2sHash::from("f308fc02ce9172ad02a7d75800ecfc027109bc67987ea32aba9b8dcc7b10150e")
    );
    let mut h = Blake2sHasher::default();
    h.write_all(b"te").unwrap();
    h.write_all(b"st").unwrap();
    assert_eq!(
        h.finish(),
        Blake2sHash::from("f308fc02ce9172ad02a7d75800ecfc027109bc67987ea32aba9b8dcc7b10150e")
    );
}

#[test]
fn it_can_compute_sha512() {
    // sha512('test') = 'ee26b0dd4af7e749aa1a8ee3c10ae9923f618980772e473f8819a5d4940e0db27ac185f8a0e1d5f84f88bc887fd67b143732c304cc5fa9ad8e6f57f50028a8ff'

    assert_eq!(
        Sha512Hasher::default().digest(b"test"),
        Sha512Hash::from("ee26b0dd4af7e749aa1a8ee3c10ae9923f618980772e473f8819a5d4940e0db27ac185f8a0e1d5f84f88bc887fd67b143732c304cc5fa9ad8e6f57f50028a8ff")
    );
    let mut h = Sha512Hasher::default();
    h.write_all(b"te").unwrap();
    h.write_all(b"st").unwrap();
    assert_eq!(
        h.finish(),
        Sha512Hash::from("ee26b0dd4af7e749aa1a8ee3c10ae9923f618980772e473f8819a5d4940e0db27ac185f8a0e1d5f84f88bc887fd67b143732c304cc5fa9ad8e6f57f50028a8ff")
    );
}

#[test]
fn it_can_compute_argon2_kdf() {
    let password = "test";
    let salt = "nimiqrocks!";

    let res2d = argon2kdf::compute_argon2_kdf(
        password.as_bytes(),
        salt.as_bytes(),
        1,
        32,
        argon2::Variant::Argon2d,
    );
    assert_eq!(
        res2d.unwrap(),
        hex::decode("8c259fdcc2ad6799df728c11e895a3369e9dbae6a3166ebc3b353399fc565524").unwrap()
    );

    let res2id = argon2kdf::compute_argon2_kdf(
        password.as_bytes(),
        salt.as_bytes(),
        1,
        32,
        argon2::Variant::Argon2id,
    );
    assert_eq!(
        res2id.unwrap(),
        hex::decode("8d8a0ac8da6f305cfc505411db3d3d17cda3aa1773e4c63b85aade07fdfa637e").unwrap()
    );
}

#[test]
fn it_can_compute_keccak256() {
    // keccak256('test') = '9c22ff5f21f0b81b113e63f7db6da94fedef11b2119b4088b89664fb9a3cb658'
    // Test vector from: https://emn178.github.io/online-tools/keccak_256.html

    assert_eq!(
        Keccak256Hasher::default().digest(b"test"),
        Keccak256Hash::from("9c22ff5f21f0b81b113e63f7db6da94fedef11b2119b4088b89664fb9a3cb658")
    );
    let mut h = Keccak256Hasher::default();
    h.write_all(b"te").unwrap();
    h.write_all(b"st").unwrap();
    assert_eq!(
        h.finish(),
        Keccak256Hash::from("9c22ff5f21f0b81b113e63f7db6da94fedef11b2119b4088b89664fb9a3cb658")
    );
}

#[test]
fn it_can_compute_keccak256_empty() {
    // keccak256('') = 'c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470'
    assert_eq!(
        Keccak256Hasher::default().digest(b""),
        Keccak256Hash::from("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
    );
}

#[test]
fn it_can_merge_keccak256_empty() {
    // Test Merge::empty() with various prefixes
    let prefix1: u64 = 0;
    let hash1 = Keccak256Hash::empty(prefix1);
    assert_eq!(hash1.as_bytes().len(), 32);

    let prefix2: u64 = 1;
    let hash2 = Keccak256Hash::empty(prefix2);
    assert_eq!(hash2.as_bytes().len(), 32);

    // Different prefixes should produce different hashes
    assert_ne!(hash1, hash2);
}

#[test]
fn it_can_merge_keccak256_hashes() {
    // Test Merge::merge() with sample data
    let hash1 = Keccak256Hasher::default().digest(b"hello");
    let hash2 = Keccak256Hasher::default().digest(b"world");

    let prefix: u64 = 42;
    let merged = hash1.merge(&hash2, prefix);

    // Verify the merged hash has correct length
    assert_eq!(merged.as_bytes().len(), 32);

    // Verify merging is deterministic
    let merged2 = hash1.merge(&hash2, prefix);
    assert_eq!(merged, merged2);

    // Verify different prefixes produce different results
    let merged_different_prefix = hash1.merge(&hash2, 43);
    assert_ne!(merged, merged_different_prefix);

    // Verify order matters
    let merged_reversed = hash2.merge(&hash1, prefix);
    assert_ne!(merged, merged_reversed);
}

#[test]
fn it_can_hex_encode_decode_keccak256() {
    let original = Keccak256Hasher::default().digest(b"test");
    let hex_string = hex::encode(original.as_bytes());
    assert_eq!(
        hex_string,
        "9c22ff5f21f0b81b113e63f7db6da94fedef11b2119b4088b89664fb9a3cb658"
    );

    // Test decoding from hex string
    let decoded =
        Keccak256Hash::from("9c22ff5f21f0b81b113e63f7db6da94fedef11b2119b4088b89664fb9a3cb658");
    assert_eq!(original, decoded);
}

#[test]
fn it_can_serialize_deserialize_keccak256() {
    use nimiq_serde::{Deserialize, Serialize};

    let original = Keccak256Hasher::default().digest(b"test");
    let serialized = original.serialize_to_vec();
    let deserialized: Keccak256Hash = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(original, deserialized);
}
