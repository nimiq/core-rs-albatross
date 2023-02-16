use std::str::FromStr;

use wasm_bindgen::prelude::*;

use crate::address::Address;
use crate::private_key::PrivateKey;
use crate::signature::Signature;

#[wasm_bindgen]
pub struct PublicKey {
    inner: nimiq_keys::PublicKey,
}

#[wasm_bindgen]
impl PublicKey {
    pub fn derive(private_key: &PrivateKey) -> PublicKey {
        PublicKey::from_native(nimiq_keys::PublicKey::from(private_key.native_ref()))
    }

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        self.inner.verify(signature.native_ref(), data)
    }

    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: &[u8]) -> Result<PublicKey, JsError> {
        match nimiq_keys::PublicKey::from_bytes(bytes) {
            Ok(key) => Ok(PublicKey::from_native(key)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }

    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<PublicKey, JsError> {
        match nimiq_keys::PublicKey::from_str(hex) {
            Ok(key) => Ok(PublicKey::from_native(key)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }

    #[wasm_bindgen(js_name = toAddress)]
    pub fn to_address(&self) -> Address {
        Address::from_native(nimiq_keys::Address::from(&self.inner))
    }
}

impl PublicKey {
    pub fn from_native(public_key: nimiq_keys::PublicKey) -> PublicKey {
        PublicKey { inner: public_key }
    }

    pub fn native_ref(&self) -> &nimiq_keys::PublicKey {
        &self.inner
    }
}
