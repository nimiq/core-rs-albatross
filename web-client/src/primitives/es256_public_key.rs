use std::str::FromStr;

use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

use crate::{address::Address, primitives::es256_signature::ES256Signature};

/// The non-secret (public) part of an ES256 asymmetric key pair that is typically used to digitally verify or encrypt data.
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct ES256PublicKey {
    inner: nimiq_keys::ES256PublicKey,
}

impl ES256PublicKey {
    const SPKI_SIZE: usize = 91;
    const RAW_SIZE: usize = 65;
}

#[wasm_bindgen]
impl ES256PublicKey {
    /// Verifies that a signature is valid for this public key and the provided data.
    pub fn verify(&self, signature: &ES256Signature, data: &[u8]) -> bool {
        self.inner.verify(signature.native_ref(), data)
    }

    /// Deserializes a public key from a byte array.
    ///
    /// Throws when the byte array contains less than 33 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<ES256PublicKey, JsError> {
        if bytes.len() != nimiq_keys::ES256PublicKey::SIZE {
            return Err(JsError::new("Public key primitive: Invalid length"));
        }
        let key = nimiq_keys::ES256PublicKey::deserialize_from_vec(bytes)?;
        Ok(ES256PublicKey::from(key))
    }

    /// Deserializes a public key from its SPKI representation.
    #[wasm_bindgen(js_name = fromSpki)]
    pub fn from_spki(spki_bytes: &[u8]) -> Result<ES256PublicKey, JsError> {
        if spki_bytes.len() != Self::SPKI_SIZE {
            return Err(JsError::new("Public key primitive: Invalid SPKI length"));
        }
        // The raw key is the last 65 bytes of the SPKI format
        let raw_key = &spki_bytes[spki_bytes.len() - Self::RAW_SIZE..];
        Self::from_raw(raw_key)
    }

    /// Deserializes a public key from its raw representation.
    #[wasm_bindgen(js_name = fromRaw)]
    pub fn from_raw(raw_bytes: &[u8]) -> Result<ES256PublicKey, JsError> {
        if raw_bytes.len() != Self::RAW_SIZE {
            return Err(JsError::new("Public key primitive: Invalid raw length"));
        }
        // Take the first byte (parity) and the X coordinate of the key (32 bytes)
        let mut compressed = raw_bytes[0..33].to_vec();
        // Adjust the parity byte according to the Y coordinate
        compressed[0] = 0x02 | (raw_bytes[raw_bytes.len() - 1] & 0x01);
        Self::unserialize(&compressed)
    }

    /// Creates a new public key from a byte array.
    ///
    /// Compatible with the `-7` COSE algorithm identifier.
    ///
    /// ## Example
    ///
    /// ```javascript
    /// // Create/register a credential with the Webauthn API:
    /// const cred = await navigator.credentials.create({
    ///     publicKey: {
    ///         pubKeyCredParams: [{
    ///             type: "public-key",
    ///             alg: -7, // ES256 = ECDSA over P-256 with SHA-256
    ///        }],
    ///        // ...
    ///     },
    /// });
    ///
    /// // Then create an instance of ES256PublicKey from the credential response:
    /// const publicKey = new Nimiq.ES256PublicKey(new Uint8Array(cred.response.getPublicKey()));
    /// ```
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<ES256PublicKey, JsError> {
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
    /// Throws when the string is not valid hex format or when it represents less than 33 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<ES256PublicKey, JsError> {
        let key = nimiq_keys::ES256PublicKey::from_str(hex)?;
        Ok(ES256PublicKey::from(key))
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

impl From<nimiq_keys::ES256PublicKey> for ES256PublicKey {
    fn from(public_key: nimiq_keys::ES256PublicKey) -> ES256PublicKey {
        ES256PublicKey { inner: public_key }
    }
}

impl ES256PublicKey {
    pub fn native_ref(&self) -> &nimiq_keys::ES256PublicKey {
        &self.inner
    }
}
