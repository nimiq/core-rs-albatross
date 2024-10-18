use std::str::FromStr;

use nimiq_keys::SecureGenerate;
use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// The secret (private) part of an asymmetric key pair that is typically used to digitally sign or decrypt data.
#[wasm_bindgen]
pub struct PrivateKey {
    inner: nimiq_keys::PrivateKey,
}

#[wasm_bindgen]
impl PrivateKey {
    #[wasm_bindgen(getter = PURPOSE_ID)]
    pub fn purpose_id() -> u32 {
        0x42000001
    }

    #[wasm_bindgen(getter = SIZE)]
    pub fn size() -> usize {
        nimiq_keys::PrivateKey::SIZE
    }

    #[wasm_bindgen(getter = serializedSize)]
    pub fn serialized_size(&self) -> usize {
        PrivateKey::size()
    }

    /// Generates a new private key from secure randomness.
    pub fn generate() -> PrivateKey {
        PrivateKey::from(nimiq_keys::PrivateKey::generate_default_csprng())
    }

    /// Deserializes a private key from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<PrivateKey, JsError> {
        let key = nimiq_keys::PrivateKey::deserialize_from_vec(bytes)?;
        Ok(PrivateKey::from(key))
    }

    /// Creates a new private key from a byte array.
    ///
    /// Throws when the byte array is not exactly 32 bytes long.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<PrivateKey, JsError> {
        if bytes.len() != nimiq_keys::PrivateKey::SIZE {
            return Err(JsError::new("Private key primitive: Invalid length"));
        }
        Self::deserialize(bytes)
    }

    /// Serializes the private key to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Parses a private key from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 32 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<PrivateKey, JsError> {
        let key = nimiq_keys::PrivateKey::from_str(hex)?;
        Ok(PrivateKey::from(key))
    }

    /// Formats the private key into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl From<nimiq_keys::PrivateKey> for PrivateKey {
    fn from(private_key: nimiq_keys::PrivateKey) -> PrivateKey {
        PrivateKey { inner: private_key }
    }
}

impl PrivateKey {
    pub fn native_ref(&self) -> &nimiq_keys::PrivateKey {
        &self.inner
    }
}
