use std::str::FromStr;

use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::{address::Address, primitives::signature::Signature};

/// The non-secret (public) part of an ES256 asymmetric key pair that is typically used to digitally verify or encrypt data.
#[wasm_bindgen]
pub struct WebauthnPublicKey {
    inner: nimiq_keys::WebauthnPublicKey,
}

#[wasm_bindgen]
impl WebauthnPublicKey {
    /// Verifies that a signature is valid for this public key and the provided data.
    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        self.inner.verify(signature.native_ref(), data)
    }

    /// Deserializes a public key from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<WebauthnPublicKey, JsError> {
        let key = nimiq_keys::WebauthnPublicKey::deserialize_from_vec(bytes)?;
        Ok(WebauthnPublicKey::from_native(key))
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
    /// // Then create an instance of WebauthnPublicKey from the credential response:
    /// const publicKey = new Nimiq.WebauthnPublicKey(new Uint8Array(cred.response.getPublicKey()));
    /// ```
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<WebauthnPublicKey, JsError> {
        if bytes.len() != nimiq_keys::WebauthnPublicKey::SIZE {
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
    pub fn from_hex(hex: &str) -> Result<WebauthnPublicKey, JsError> {
        let key = nimiq_keys::WebauthnPublicKey::from_str(hex)?;
        Ok(WebauthnPublicKey::from_native(key))
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

impl WebauthnPublicKey {
    pub fn from_native(public_key: nimiq_keys::WebauthnPublicKey) -> WebauthnPublicKey {
        WebauthnPublicKey { inner: public_key }
    }

    pub fn native_ref(&self) -> &nimiq_keys::WebauthnPublicKey {
        &self.inner
    }
}
