use std::str::FromStr;

use wasm_bindgen::prelude::*;

/// A ES256 Signature represents a cryptocraphic proof that an ES256 private key signed some data.
/// It can be verified with the private key's public key.
#[wasm_bindgen]
pub struct ES256Signature {
    inner: nimiq_keys::ES256Signature,
}

#[wasm_bindgen]
impl ES256Signature {
    /// Deserializes a ES256 Signature from a byte array.
    ///
    /// Throws when the byte array contains less than 64 bytes.
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn deserialize(bytes: &[u8]) -> Result<ES256Signature, JsError> {
        match nimiq_keys::ES256Signature::from_bytes(bytes) {
            Ok(sig) => Ok(ES256Signature::from_native(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Serializes the signature to a byte array.
    #[wasm_bindgen(js_name = toBytes)]
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.to_bytes().to_vec()
    }

    #[wasm_bindgen(js_name = fromAsn1)]
    pub fn from_asn1(bytes: &[u8]) -> Result<ES256Signature, JsError> {
        if bytes.len() < 70 || bytes.len() > 72 {
            return Err(JsError::new(
                format!(
                    "Invalid ASN.1 bytes length: Expected between 70 and 72, got {}",
                    bytes.len()
                )
                .as_str(),
            ));
        }

        // Convert signature serialization from ASN.1 sequence to "raw" format
        let r_start = if bytes[4] == 0 { 5 } else { 4 };
        let r_end = r_start + 32;
        let s_start = if bytes[r_end + 2] == 0 {
            r_end + 3
        } else {
            r_end + 2
        };
        let r = &bytes[r_start..r_end];
        let s = &bytes[s_start..];
        let signature = [r, s].concat();

        ES256Signature::deserialize(signature.as_slice())
    }

    /// Parses a ES256 Signature from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 64 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<ES256Signature, JsError> {
        match nimiq_keys::ES256Signature::from_str(hex) {
            Ok(sig) => Ok(ES256Signature::from_native(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Formats the signature into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl ES256Signature {
    pub fn from_native(signature: nimiq_keys::ES256Signature) -> ES256Signature {
        ES256Signature { inner: signature }
    }

    pub fn native_ref(&self) -> &nimiq_keys::ES256Signature {
        &self.inner
    }
}
