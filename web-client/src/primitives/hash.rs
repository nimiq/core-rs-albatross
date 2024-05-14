use nimiq_hash::argon2kdf::{compute_argon2_kdf, Argon2Variant};
use nimiq_serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Hash;

#[wasm_bindgen]
impl Hash {
    /// Computes a 32-byte Blake2b hash from the input data. Blake2b is used for example to compute a public key's address.
    #[wasm_bindgen(js_name = computeBlake2b)]
    pub fn compute_blake2b(data: &[u8]) -> Vec<u8> {
        nimiq_hash::Hasher::digest(nimiq_hash::Blake2bHasher::default(), data).serialize_to_vec()
    }

    /// Computes a 32-byte SHA256 hash from the input data.
    #[wasm_bindgen(js_name = computeSha256)]
    pub fn compute_sha256(data: &[u8]) -> Vec<u8> {
        nimiq_hash::Hasher::digest(nimiq_hash::Sha256Hasher::default(), data).serialize_to_vec()
    }

    /// Computes a 64-byte SHA512 hash from the input data.
    #[wasm_bindgen(js_name = computeSha512)]
    pub fn compute_sha512(data: &[u8]) -> Vec<u8> {
        nimiq_hash::Hasher::digest(nimiq_hash::sha512::Sha512Hasher::default(), data)
            .serialize_to_vec()
    }

    #[wasm_bindgen(js_name = computeNimiqArgon2d)]
    pub fn compute_nimiq_argon2d(
        password: &[u8],
        salt: &[u8],
        iterations: u32,
        derived_key_length: usize,
    ) -> Result<Vec<u8>, JsError> {
        Ok(compute_argon2_kdf(
            password,
            salt,
            iterations,
            derived_key_length,
            Argon2Variant::Argon2d,
        )?)
    }

    #[wasm_bindgen(js_name = computeNimiqArgon2id)]
    pub fn compute_nimiq_argon2id(
        password: &[u8],
        salt: &[u8],
        iterations: u32,
        derived_key_length: usize,
    ) -> Result<Vec<u8>, JsError> {
        Ok(compute_argon2_kdf(
            password,
            salt,
            iterations,
            derived_key_length,
            Argon2Variant::Argon2id,
        )?)
    }
}
