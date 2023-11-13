use std::str::FromStr;

use wasm_bindgen::prelude::*;

/// A signature represents a cryptocraphic proof that a private key signed some data.
/// It can be verified with the private key's public key.
#[wasm_bindgen]
pub struct Signature {
    inner: nimiq_keys::Ed25519Signature,
}

#[wasm_bindgen]
impl Signature {
    /// Deserializes a signature from a byte array.
    ///
    /// Throws when the byte array contains less than 64 bytes.
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn deserialize(bytes: &[u8]) -> Result<Signature, JsError> {
        match nimiq_keys::Ed25519Signature::from_bytes(bytes) {
            Ok(sig) => Ok(Signature::from_native(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Serializes the signature to a byte array.
    #[wasm_bindgen(js_name = toBytes)]
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.to_bytes().to_vec()
    }

    /// Parses a signature from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 64 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<Signature, JsError> {
        match nimiq_keys::Ed25519Signature::from_str(hex) {
            Ok(sig) => Ok(Signature::from_native(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Formats the signature into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl Signature {
    pub fn from_native(signature: nimiq_keys::Ed25519Signature) -> Signature {
        Signature { inner: signature }
    }

    pub fn native_ref(&self) -> &nimiq_keys::Ed25519Signature {
        &self.inner
    }
}
