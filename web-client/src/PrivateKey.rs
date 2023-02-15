use std::str::FromStr;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct PrivateKey {
    inner: nimiq_keys::PrivateKey,
}

#[wasm_bindgen]
impl PrivateKey {
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<PrivateKey, JsError> {
        match nimiq_keys::PrivateKey::from_str(hex) {
            Ok(key) => Ok(PrivateKey { inner: key }),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}
