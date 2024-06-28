use std::str::FromStr;

use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

use crate::primitives::{private_key::PrivateKey, public_key::PublicKey};

/// An Ed25519 Signature represents a cryptographic proof that a private key signed some data.
/// It can be verified with the private key's public key.
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct Signature {
    inner: nimiq_keys::Ed25519Signature,
}

#[wasm_bindgen]
impl Signature {
    /// Deserializes an Ed25519 signature from a byte array.
    ///
    /// Throws when the byte array contains less than 64 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<Signature, JsError> {
        match nimiq_keys::Ed25519Signature::from_bytes(bytes) {
            Ok(sig) => Ok(Signature::from(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Serializes the signature to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.to_bytes().to_vec()
    }

    /// Create a signature from a private key and its public key over byte data.
    pub fn create(private_key: &PrivateKey, public_key: &PublicKey, data: &[u8]) -> Signature {
        let key_pair = nimiq_keys::KeyPair {
            public: *public_key.native_ref(),
            private: private_key.native_ref().clone(),
        };
        let signature = key_pair.sign(data);
        Signature::from(signature)
    }

    /// Parses an Ed25519 signature from its ASN.1 representation.
    #[wasm_bindgen(js_name = fromAsn1)]
    pub fn from_asn1(bytes: &[u8]) -> Result<Signature, JsError> {
        Signature::unserialize(Signature::asn1_to_raw(bytes)?.as_slice())
    }

    /// Parses an Ed25519 signature from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 64 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<Signature, JsError> {
        match nimiq_keys::Ed25519Signature::from_str(hex) {
            Ok(sig) => Ok(Signature::from(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    /// Formats the signature into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl From<nimiq_keys::Ed25519Signature> for Signature {
    fn from(signature: nimiq_keys::Ed25519Signature) -> Signature {
        Signature { inner: signature }
    }
}

impl Signature {
    pub fn native_ref(&self) -> &nimiq_keys::Ed25519Signature {
        &self.inner
    }

    pub fn asn1_to_raw(bytes: &[u8]) -> Result<Vec<u8>, JsError> {
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
        Ok([r, s].concat())
    }
}
