use beserial::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use super::secret_key::BLSSecretKey;

/// The public part of the BLS keypair.
/// This is specified in the staking contract to verify votes from Validators.
#[wasm_bindgen]
pub struct BLSPublicKey {
    inner: nimiq_bls::PublicKey,
}

#[wasm_bindgen]
impl BLSPublicKey {
    /// Derives a public key from an existing private key.
    pub fn derive(secret_key: &BLSSecretKey) -> BLSPublicKey {
        BLSPublicKey::from_native(nimiq_bls::PublicKey::from_secret(secret_key.native_ref()))
    }

    /// Deserializes a public key from a byte array.
    pub fn unserialize(bytes: &[u8]) -> Result<BLSPublicKey, JsError> {
        let key = nimiq_bls::PublicKey::deserialize(&mut &*bytes)?;
        Ok(BLSPublicKey::from_native(key))
    }

    /// Creates a new public key from a byte array.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<BLSPublicKey, JsError> {
        Self::unserialize(bytes)
    }

    /// Serializes the public key to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Parses a public key from its hex representation.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<BLSPublicKey, JsError> {
        let raw = hex::decode(hex)?;

        BLSPublicKey::unserialize(&raw)
    }

    /// Formats the public key into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        let vec = BLSPublicKey::serialize(&self);
        hex::encode(vec)
    }
}

impl BLSPublicKey {
    pub fn from_native(public_key: nimiq_bls::PublicKey) -> BLSPublicKey {
        BLSPublicKey { inner: public_key }
    }

    pub fn native_ref(&self) -> &nimiq_bls::PublicKey {
        &self.inner
    }
}
