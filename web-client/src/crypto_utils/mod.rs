#[cfg(feature = "crypto")]
use js_sys::Uint8Array;
#[cfg(feature = "primitives")]
use nimiq_hash::{hmac::compute_hmac_sha512, pbkdf2::compute_pbkdf2_sha512};
#[cfg(feature = "crypto")]
use nimiq_utils::otp::{otp, Algorithm};
#[cfg(feature = "primitives")]
use rand_core::RngCore;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct CryptoUtils;

#[wasm_bindgen]
impl CryptoUtils {
    #[cfg(feature = "primitives")]
    /// Generates a secure random byte array of the given length.
    #[wasm_bindgen(js_name = getRandomValues)]
    pub fn get_random_values(length: usize) -> Vec<u8> {
        let mut buf = vec![0u8; length];
        rand_core::OsRng.fill_bytes(&mut buf);
        buf
    }

    #[cfg(feature = "primitives")]
    /// Computes a 64-byte [HMAC]-SHA512 hash from the input key and data.
    ///
    /// [HMAC]: https://en.wikipedia.org/wiki/HMAC
    #[wasm_bindgen(js_name = computeHmacSha512)]
    pub fn compute_hmac_sha512(key: &[u8], data: &[u8]) -> Vec<u8> {
        compute_hmac_sha512(key, data).as_bytes().to_vec()
    }

    #[cfg(feature = "primitives")]
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

    #[cfg(feature = "crypto")]
    /// Encrypts a message with an [OTP] [KDF] and the given parameters.
    /// The KDF uses Argon2d for hashing.
    ///
    /// [OTP]: https://en.wikipedia.org/wiki/One-time_pad
    /// [KDF]: https://en.wikipedia.org/wiki/Key_derivation_function
    #[wasm_bindgen(js_name = otpKdf)]
    pub async fn otp_kdf(
        message: &[u8],
        key: &[u8],
        salt: &[u8],
        iterations: u32,
    ) -> Result<Uint8Array, JsError> {
        Ok(Uint8Array::from(
            otp(message, key, iterations, salt, Algorithm::Argon2d)
                .map_err(|err| JsError::new(&format!("{:?}", err)))?
                .as_slice(),
        ))
    }
}
