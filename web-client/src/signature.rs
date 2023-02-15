use std::str::FromStr;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Signature {
    inner: nimiq_keys::Signature,
}

#[wasm_bindgen]
impl Signature {
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: &[u8]) -> Result<Signature, JsError> {
        match nimiq_keys::Signature::from_bytes(bytes) {
            Ok(sig) => Ok(Signature::from_native(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }

    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<Signature, JsError> {
        match nimiq_keys::Signature::from_str(hex) {
            Ok(sig) => Ok(Signature::from_native(sig)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl Signature {
    pub fn from_native(signature: nimiq_keys::Signature) -> Signature {
        Signature { inner: signature }
    }

    pub fn to_native(&self) -> nimiq_keys::Signature {
        self.inner
    }
}
