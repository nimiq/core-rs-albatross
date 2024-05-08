use std::io::Write;

use nimiq_hash::{
    argon2kdf,
    sha512::{Sha512Hash, Sha512Hasher},
    Blake2bHash, Blake2bHasher, Blake2sHash, Blake2sHasher, Hasher, Sha256Hash, Sha256Hasher,
};
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

    let res = argon2kdf::compute_argon2_kdf(
        password.as_bytes(),
        salt.as_bytes(),
        1,
        32,
        argon2::Variant::Argon2d,
    );
    assert_eq!(
        res.unwrap(),
        hex::decode("8c259fdcc2ad6799df728c11e895a3369e9dbae6a3166ebc3b353399fc565524").unwrap()
    )
}
