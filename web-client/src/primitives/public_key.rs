use std::str::FromStr;

use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::{
    address::Address,
    primitives::{private_key::PrivateKey, signature::Signature},
};

/// The non-secret (public) part of an asymmetric key pair that is typically used to digitally verify or encrypt data.
#[wasm_bindgen]
pub struct PublicKey {
    inner: nimiq_keys::EdDSAPublicKey,
}

#[wasm_bindgen]
impl PublicKey {
    /// Derives a public key from an existing private key.
    pub fn derive(private_key: &PrivateKey) -> PublicKey {
        PublicKey::from_native(nimiq_keys::EdDSAPublicKey::from(private_key.native_ref()))
    }

    /// Verifies that a signature is valid for this public key and the provided data.
    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        self.inner.verify(signature.native_ref(), data)
    }

    /// Deserializes a public key from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<PublicKey, JsError> {
        let key = nimiq_keys::EdDSAPublicKey::deserialize_from_vec(bytes)?;
        Ok(PublicKey::from_native(key))
    }

    /// Creates a new public key from a byte array.
    ///
    /// Throws when the byte array is not exactly 32 bytes long.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<PublicKey, JsError> {
        if bytes.len() != nimiq_keys::EdDSAPublicKey::SIZE {
            return Err(JsError::new("Public key primitive: Invalid length"));
        }
        Self::unserialize(bytes)
    }

    /// Serializes the public key to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Parses a public key from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 32 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<PublicKey, JsError> {
        let key = nimiq_keys::EdDSAPublicKey::from_str(hex)?;
        Ok(PublicKey::from_native(key))
    }

    /// Formats the public key into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }

    /// Gets the public key's address.
    #[wasm_bindgen(js_name = toAddress)]
    pub fn to_address(&self) -> Address {
        Address::from_native(nimiq_keys::Address::from(&self.inner))
    }
}

impl PublicKey {
    pub fn from_native(public_key: nimiq_keys::EdDSAPublicKey) -> PublicKey {
        PublicKey { inner: public_key }
    }

    pub fn native_ref(&self) -> &nimiq_keys::EdDSAPublicKey {
        &self.inner
    }
}
