use std::str::FromStr;

use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

/// An object representing a Nimiq address.
/// Offers methods to parse and format addresses from and to strings.
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct Address {
    inner: nimiq_keys::Address,
}

#[wasm_bindgen]
impl Address {
    /// Parses an address from an {@link Address} instance or a string representation.
    ///
    /// Throws when an address cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(addr: &AddressAnyType) -> Result<Address, JsError> {
        let js_value: &JsValue = addr.unchecked_ref();

        Address::try_from(js_value).or_else(|_| {
            Address::from_string(&serde_wasm_bindgen::from_value::<String>(
                js_value.to_owned(),
            )?)
        })
    }

    /// Parses an address from a string representation, either user-friendly or hex format.
    ///
    /// Throws when an address cannot be parsed from the string.
    #[wasm_bindgen(js_name = fromString)]
    pub fn from_string(str: &str) -> Result<Address, JsError> {
        Ok(Address::from_native(nimiq_keys::Address::from_str(str)?))
    }

    /// Parses an address from its user-friendly string representation.
    ///
    /// Throws when an address cannot be parsed from the string.
    #[wasm_bindgen(js_name = fromUserFriendlyAddress)]
    pub fn from_user_friendly_address(str: &str) -> Result<Address, JsError> {
        Ok(Address::from_native(
            nimiq_keys::Address::from_user_friendly_address(str)?,
        ))
    }

    /// Formats the address into a plain string format.
    #[wasm_bindgen(js_name = toPlain)]
    pub fn to_plain(&self) -> String {
        self.to_user_friendly_address()
    }

    /// Formats the address into user-friendly IBAN format.
    #[wasm_bindgen(js_name = toUserFriendlyAddress)]
    pub fn to_user_friendly_address(&self) -> String {
        self.inner.to_user_friendly_address()
    }

    /// Formats the address into hex format.
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

    pub fn native(&self) -> nimiq_keys::Address {
        self.inner.clone()
    }

    pub fn take_native(self) -> nimiq_keys::Address {
        self.inner
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Address | string")]
    pub type AddressAnyType;
}
