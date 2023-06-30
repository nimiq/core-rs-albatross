use beserial::{Deserialize, Serialize};
use nimiq_keys::SecureGenerate;
use wasm_bindgen::prelude::*;

/// The secret part of the BLS keypair.
/// This is specified in the config file, and is used by Validators to vote.
#[wasm_bindgen]
pub struct BLSSecretKey {
    inner: nimiq_bls::SecretKey,
}

#[wasm_bindgen]
impl BLSSecretKey {
    /// Generates a new private key from secure randomness.
    pub fn generate() -> BLSSecretKey {
        BLSSecretKey::from_native(nimiq_bls::SecretKey::generate_default_csprng())
    }

    /// Deserializes a private key from a byte array.
    pub fn unserialize(bytes: &[u8]) -> Result<BLSSecretKey, JsError> {
        let key = nimiq_bls::SecretKey::deserialize(&mut &*bytes)?;
        Ok(BLSSecretKey::from_native(key))
    }

    /// Creates a new private key from a byte array.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<BLSSecretKey, JsError> {
        if bytes.len() != nimiq_bls::SecretKey::SIZE {
            return Err(JsError::new("BLS Secret key primitive: Invalid length"));
        }
        Self::unserialize(bytes)
    }

    /// Serializes the private key to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Parses a private key from its hex representation.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<BLSSecretKey, JsError> {
        let raw = hex::decode(hex)?;
        // 95 and 96 byres are valid sizes.
        if !(raw.len() == nimiq_bls::SecretKey::SIZE
            || raw.len() + 1 as usize == nimiq_bls::SecretKey::SIZE)
        {
            return Err(JsError::new(
                format!("BLS Secret key primitive: Invalid length: {}", raw.len()).as_str(),
            ));
        }

        BLSSecretKey::unserialize(&raw)
    }

    /// Formats the private key into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        let vec = BLSSecretKey::serialize(&self);
        hex::encode(vec)
    }
}

impl BLSSecretKey {
    pub fn from_native(secret_key: nimiq_bls::SecretKey) -> BLSSecretKey {
        BLSSecretKey { inner: secret_key }
    }

    pub fn native_ref(&self) -> &nimiq_bls::SecretKey {
        &self.inner
    }
}
