use std::str::FromStr;

use nimiq_keys::SecureGenerate;
use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::{
    address::Address,
    primitives::{private_key::PrivateKey, public_key::PublicKey, signature::Signature},
    transaction::Transaction,
};

/// A keypair represents a private key and its respective public key.
/// It is used for signing data, usually transactions.
#[wasm_bindgen]
pub struct KeyPair {
    inner: nimiq_keys::KeyPair,
}

#[wasm_bindgen]
impl KeyPair {
    /// Generates a new keypair from secure randomness.
    pub fn generate() -> KeyPair {
        let key_pair = nimiq_keys::KeyPair::generate_default_csprng();
        KeyPair::from_native(key_pair)
    }

    /// Derives a keypair from an existing private key.
    pub fn derive(private_key: &PrivateKey) -> KeyPair {
        let key_pair = nimiq_keys::KeyPair::from(private_key.native_ref().clone());
        KeyPair::from_native(key_pair)
    }

    /// Parses a keypair from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 64 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<KeyPair, JsError> {
        let private = nimiq_keys::PrivateKey::from_str(&hex[0..64])?;
        let public = nimiq_keys::Ed25519PublicKey::from_str(&hex[64..])?;
        // TODO: Deserialize locked state if bytes remaining
        let key_pair = nimiq_keys::KeyPair { private, public };
        Ok(KeyPair::from_native(key_pair))
    }

    /// Deserializes a keypair from a byte array.
    ///
    /// Throws when the byte array contains less than 64 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<KeyPair, JsError> {
        let key_pair = nimiq_keys::KeyPair::deserialize_from_vec(bytes)?;
        // TODO: Deserialize locked state if bytes remaining
        Ok(KeyPair::from_native(key_pair))
    }

    #[wasm_bindgen(constructor)]
    pub fn new(private_key: &PrivateKey, public_key: &PublicKey) -> KeyPair {
        let key_pair = nimiq_keys::KeyPair {
            private: private_key.native_ref().clone(),
            public: *public_key.native_ref(),
        };
        KeyPair::from_native(key_pair)
    }

    /// Serializes the keypair to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        let mut vec = self.inner.serialize_to_vec();
        vec.push(0); // Unlocked state (locking is not yet implemented)
        vec
    }

    /// Signs arbitrary data, returns a signature object.
    pub fn sign(&self, data: &[u8]) -> Signature {
        Signature::from_native(self.inner.sign(data))
    }

    /// Signs a transaction and sets the signature proof on the transaction object.
    #[wasm_bindgen(js_name = signTransaction)]
    pub fn sign_transaction(&self, transaction: &mut Transaction) -> Result<(), JsError> {
        transaction.sign(self)
    }

    /// Gets the keypair's private key.
    #[wasm_bindgen(getter, js_name = privateKey)]
    pub fn private_key(&self) -> PrivateKey {
        PrivateKey::from_native(self.inner.private.clone())
    }

    /// Gets the keypair's public key.
    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> PublicKey {
        PublicKey::from_native(self.inner.public)
    }

    /// Gets the keypair's address.
    #[wasm_bindgen(js_name = toAddress)]
    pub fn to_address(&self) -> Address {
        Address::from_native(nimiq_keys::Address::from(&self.inner))
    }

    /// Formats the keypair into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }
}

impl KeyPair {
    pub fn from_native(key_pair: nimiq_keys::KeyPair) -> KeyPair {
        KeyPair { inner: key_pair }
    }

    pub fn native_ref(&self) -> &nimiq_keys::KeyPair {
        &self.inner
    }
}
