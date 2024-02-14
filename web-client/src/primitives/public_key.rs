use std::str::FromStr;

use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

use crate::{
    address::Address,
    primitives::{private_key::PrivateKey, signature::Signature},
};

/// The non-secret (public) part of an asymmetric key pair that is typically used to digitally verify or encrypt data.
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct PublicKey {
    inner: nimiq_keys::Ed25519PublicKey,
}

impl PublicKey {
    const SPKI_SIZE: usize = 44;
    const RAW_SIZE: usize = 32;
}

#[wasm_bindgen]
impl PublicKey {
    /// Derives a public key from an existing private key.
    pub fn derive(private_key: &PrivateKey) -> PublicKey {
        PublicKey::from(nimiq_keys::Ed25519PublicKey::from(private_key.native_ref()))
    }

    /// Verifies that a signature is valid for this public key and the provided data.
    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        self.inner.verify(signature.native_ref(), data)
    }

    /// Deserializes a public key from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<PublicKey, JsError> {
        let key = nimiq_keys::Ed25519PublicKey::deserialize_from_vec(bytes)?;
        Ok(PublicKey::from(key))
    }

    /// Deserializes a public key from its SPKI representation.
    #[wasm_bindgen(js_name = fromSpki)]
    pub fn from_spki(spki_bytes: &[u8]) -> Result<PublicKey, JsError> {
        if spki_bytes.len() != Self::SPKI_SIZE {
            return Err(JsError::new("Public key primitive: Invalid SPKI length"));
        }
        // The raw key is the last 32 bytes of the SPKI format
        let raw_key = &spki_bytes[spki_bytes.len() - Self::RAW_SIZE..];
        Self::from_raw(raw_key)
    }

    /// Deserializes a public key from its raw representation.
    #[wasm_bindgen(js_name = fromRaw)]
    pub fn from_raw(raw_bytes: &[u8]) -> Result<PublicKey, JsError> {
        if raw_bytes.len() != Self::RAW_SIZE {
            return Err(JsError::new("Public key primitive: Invalid raw length"));
        }
        Self::unserialize(raw_bytes)
    }

    /// Creates a new public key from a byte array.
    ///
    /// Throws when the byte array is not exactly 32 bytes long.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<PublicKey, JsError> {
        if bytes.len() == Self::SPKI_SIZE {
            return Self::from_spki(bytes);
        }
        if bytes.len() == Self::RAW_SIZE {
            return Self::from_raw(bytes);
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
        let key = nimiq_keys::Ed25519PublicKey::from_str(hex)?;
        Ok(PublicKey::from(key))
    }

    /// Formats the public key into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }

    /// Gets the public key's address.
    #[wasm_bindgen(js_name = toAddress)]
    pub fn to_address(&self) -> Address {
        Address::from(nimiq_keys::Address::from(&self.inner))
    }
}

impl From<nimiq_keys::Ed25519PublicKey> for PublicKey {
    fn from(public_key: nimiq_keys::Ed25519PublicKey) -> PublicKey {
        PublicKey { inner: public_key }
    }
}

impl PublicKey {
    pub fn native_ref(&self) -> &nimiq_keys::Ed25519PublicKey {
        &self.inner
    }
}
