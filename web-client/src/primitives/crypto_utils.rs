use nimiq_hash::{hmac::compute_hmac_sha512, pbkdf2::compute_pbkdf2_sha512};
use nimiq_utils::otp;
use otp::{otp, Algorithm};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct CryptoUtils;

#[wasm_bindgen]
impl CryptoUtils {
    /// Computes a 64-byte [HMAC]-SHA512 hash from the input key and data.
    ///
    /// [HMAC]: https://en.wikipedia.org/wiki/HMAC
    #[wasm_bindgen(js_name = computeHmacSha512)]
    pub fn compute_hmac_sha512(key: &[u8], data: &[u8]) -> Vec<u8> {
        compute_hmac_sha512(key, data).as_bytes().to_vec()
    }

    /// Computes a [PBKDF2]-over-SHA512 key from the password with the given parameters.
    ///
    /// [PBKDF2]: https://en.wikipedia.org/wiki/PBKDF2
    #[wasm_bindgen(js_name = computePBKDF2sha512)]
    pub fn compute_pbkdf2_sha512(
        password: &[u8],
        salt: &[u8],
        iterations: usize,
        derived_key_length: usize,
    ) -> Result<Vec<u8>, JsError> {
        compute_pbkdf2_sha512(password, salt, iterations, derived_key_length)
            .map_err(|err| JsError::new(&format!("{:?}", err)))
    }

    /// Encrypts a message with an [OTP] [KDF] and the given parameters.
    /// The KDF uses Argon2d for hashing.
    ///
    /// [OTP]: https://en.wikipedia.org/wiki/One-time_pad
    /// [KDF]: https://en.wikipedia.org/wiki/Key_derivation_function
    #[wasm_bindgen(js_name = otpKdf)]
    pub fn otp_kdf(
        message: &[u8],
        key: &[u8],
        salt: &[u8],
        iterations: u32,
    ) -> Result<Vec<u8>, JsError> {
        otp(message, key, iterations, salt, Algorithm::Argon2d)
            .map_err(|err| JsError::new(&format!("{:?}", err)))
    }
}
