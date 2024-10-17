use std::str::FromStr;

use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

/// An ES256 Signature represents a cryptographic proof that an ES256 private key signed some data.
/// It can be verified with the private key's public key.
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct ES256Signature {
    inner: nimiq_keys::ES256Signature,
}

#[wasm_bindgen]
impl ES256Signature {
    /// Deserializes an ES256 signature from a byte array.
    ///
    /// Throws when the byte array contains less than 64 bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<ES256Signature, JsError> {
        match nimiq_keys::ES256Signature::from_bytes(bytes) {
            Ok(sig) => Ok(ES256Signature::from(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Serializes the signature to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.to_bytes().to_vec()
    }

    /// Parses an ES256 signature from its ASN.1 representation.
    #[wasm_bindgen(js_name = fromAsn1)]
    pub fn from_asn1(bytes: &[u8]) -> Result<ES256Signature, JsError> {
        ES256Signature::deserialize(
            crate::primitives::signature::Signature::asn1_to_raw(bytes)?.as_slice(),
        )
    }

    /// Parses an ES256 signature from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 64 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<ES256Signature, JsError> {
        match nimiq_keys::ES256Signature::from_str(hex) {
            Ok(sig) => Ok(ES256Signature::from(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Formats the signature into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl From<nimiq_keys::ES256Signature> for ES256Signature {
    fn from(signature: nimiq_keys::ES256Signature) -> ES256Signature {
        ES256Signature { inner: signature }
    }
}

impl ES256Signature {
    pub fn native_ref(&self) -> &nimiq_keys::ES256Signature {
        &self.inner
    }
}
