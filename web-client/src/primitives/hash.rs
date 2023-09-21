use nimiq_serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Hash;

#[wasm_bindgen]
impl Hash {
    #[wasm_bindgen(js_name = computeBlake2b)]
    pub fn compute_blake2b(data: &[u8]) -> Vec<u8> {
        nimiq_hash::Hasher::digest(nimiq_hash::Blake2bHasher::default(), data).serialize_to_vec()
    }

    #[wasm_bindgen(js_name = computeArgon2d)]
    pub fn compute_argon2d(data: &[u8]) -> Vec<u8> {
        nimiq_hash::Hasher::digest(nimiq_hash::Argon2dHasher::default(), data).serialize_to_vec()
    }

    #[wasm_bindgen(js_name = computeSha256)]
    pub fn compute_sha256(data: &[u8]) -> Vec<u8> {
        nimiq_hash::Hasher::digest(nimiq_hash::Sha256Hasher::default(), data).serialize_to_vec()
    }

    #[wasm_bindgen(js_name = computeSha512)]
    pub fn compute_sha512(data: &[u8]) -> Vec<u8> {
        nimiq_hash::Hasher::digest(nimiq_hash::sha512::Sha512Hasher::default(), data)
            .serialize_to_vec()
    }
}
