use std::str::FromStr;

use wasm_bindgen::prelude::*;

use nimiq_keys::SecureGenerate;

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct PrivateKey {
    inner: nimiq_keys::PrivateKey,
}

#[wasm_bindgen]
impl PrivateKey {
    pub fn generate() -> PrivateKey {
        PrivateKey::from_native(nimiq_keys::PrivateKey::generate_default_csprng())
    }

    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: &[u8]) -> Result<PrivateKey, JsError> {
        match nimiq_keys::PrivateKey::from_bytes(bytes) {
            Ok(key) => Ok(PrivateKey::from_native(key)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }

    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<PrivateKey, JsError> {
        match nimiq_keys::PrivateKey::from_str(hex) {
            Ok(key) => Ok(PrivateKey::from_native(key)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl PrivateKey {
    pub fn from_native(private_key: nimiq_keys::PrivateKey) -> PrivateKey {
        PrivateKey { inner: private_key }
    }

    pub fn to_native(&self) -> nimiq_keys::PrivateKey {
        self.inner
    }
}
