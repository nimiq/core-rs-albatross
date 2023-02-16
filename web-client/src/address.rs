use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Address {
    inner: nimiq_keys::Address,
}

#[wasm_bindgen]
impl Address {
    #[wasm_bindgen(js_name = fromString)]
    pub fn from_string(string: &str) -> Result<Address, JsError> {
        match nimiq_keys::Address::from_any_str(string) {
            Ok(address) => Ok(Address::from_native(address)),
            Err(err) => Err(JsError::from(err)),
        }
    }

    #[wasm_bindgen(js_name = toUserFriendlyAddress)]
    pub fn to_user_friendly_address(&self) -> String {
        self.inner.to_user_friendly_address()
    }

    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }
}

impl Address {
    pub fn from_native(address: nimiq_keys::Address) -> Address {
        Address { inner: address }
    }

    pub fn native_ref(&self) -> &nimiq_keys::Address {
        &self.inner
    }
}
