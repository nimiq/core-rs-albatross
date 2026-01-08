use std::str::FromStr;

use js_sys::Array;
use nimiq_keys::multisig::address::combine_public_keys;
use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

use crate::{
    common::address::Address,
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
    #[wasm_bindgen(getter = SIZE)]
    pub fn size() -> usize {
        nimiq_keys::Ed25519PublicKey::SIZE
    }

    #[wasm_bindgen(getter = serializedSize)]
    pub fn serialized_size(&self) -> usize {
        PublicKey::size()
    }

    /// Derives a public key from an existing private key.
    pub fn derive(private_key: &PrivateKey) -> PublicKey {
        PublicKey::from(nimiq_keys::Ed25519PublicKey::from(private_key.native_ref()))
    }

    /// Parses a public key from a {@link PublicKey} instance, a hex string representation, or a byte array.
    ///
    /// Throws when an PublicKey cannot be parsed from the argument.
    #[cfg(feature = "primitives")]
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(key: &PublicKeyAnyType) -> Result<PublicKey, JsError> {
        let js_value: &JsValue = key.unchecked_ref();

        if let Ok(public_key) = PublicKey::try_from(js_value) {
            return Ok(public_key);
        }

        if let Ok(string) = serde_wasm_bindgen::from_value::<String>(js_value.to_owned()) {
            Ok(PublicKey::from_hex(&string)?)
        } else if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(js_value.to_owned()) {
            Ok(PublicKey::deserialize(&bytes)?)
        } else {
            Err(JsError::new("Could not parse public key"))
        }
    }

    /// Verifies that a signature is valid for this public key and the provided data.
    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        self.inner.verify(signature.native_ref(), data)
    }

    /// Deserializes a public key from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<PublicKey, JsError> {
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
        Self::deserialize(raw_bytes)
    }

    pub fn combinations(
        keys: &PublicKeyAnyArrayType,
        num_signers: usize,
    ) -> Result<Vec<PublicKey>, JsError> {
        let keys = PublicKey::unpack_public_keys(keys)?;
        let combined_keys = combine_public_keys(keys, num_signers);
        Ok(combined_keys.into_iter().map(PublicKey::from).collect())
    }

    pub fn sum(keys: &PublicKeyAnyArrayType) -> Result<PublicKey, JsError> {
        let keys = PublicKey::unpack_public_keys(keys)?;
        let combined_key =
            nimiq_keys::multisig::public_key::DelinearizedPublicKey::sum_delinearized(&keys);
        Ok(PublicKey::from(combined_key))
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
        Self::deserialize(bytes)
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

    /// Returns if this public key is equal to the other public key.
    pub fn equals(&self, other: &PublicKey) -> bool {
        self.inner == other.inner
    }

    /// Compares this public key to the other public key.
    ///
    /// Returns -1 if this public key is smaller than the other public key, 0 if they are equal,
    /// and 1 if this public key is larger than the other public key.
    pub fn compare(&self, other: &PublicKey) -> i32 {
        self.inner.cmp(&other.inner) as i32
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

    pub fn take_native(self) -> nimiq_keys::Ed25519PublicKey {
        self.inner
    }

    pub fn unpack_public_keys(
        keys: &PublicKeyAnyArrayType,
    ) -> Result<Vec<nimiq_keys::Ed25519PublicKey>, JsError> {
        // Unpack the array of keys
        let js_value: &JsValue = keys.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`keys` must be an array"))?;

        let mut keys = Vec::<_>::with_capacity(array.length().try_into()?);
        for any in array.iter() {
            let key = PublicKey::from_any(&any.into())?.take_native();
            keys.push(key);
        }

        Ok(keys)
    }
}

#[cfg(feature = "primitives")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PublicKey | string | Uint8Array")]
    pub type PublicKeyAnyType;

    #[wasm_bindgen(typescript_type = "(PublicKey | string | Uint8Array)[]")]
    pub type PublicKeyAnyArrayType;
}
