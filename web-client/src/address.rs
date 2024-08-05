use std::str::FromStr;

use nimiq_serde::Deserialize;
#[cfg(feature = "primitives")]
use nimiq_serde::Serialize;
#[cfg(feature = "primitives")]
use wasm_bindgen::convert::TryFromJsValue;
use wasm_bindgen::prelude::*;

/// An object representing a Nimiq address.
/// Offers methods to parse and format addresses from and to strings.
#[wasm_bindgen]
pub struct Address {
    inner: nimiq_keys::Address,
}

#[wasm_bindgen]
impl Address {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<Address, JsError> {
        Ok(Address::from(nimiq_keys::Address::from(
            &bytes[0..nimiq_keys::Address::len()],
        )))
    }

    /// Parses an address from an {@link Address} instance, a hex string representation, or a byte array.
    ///
    /// Throws when an address cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(addr: &AddressAnyType) -> Result<Address, JsError> {
        let js_value: &JsValue = addr.unchecked_ref();

        #[cfg(feature = "primitives")]
        if let Ok(address) = Address::try_from_js_value(js_value.to_owned()) {
            return Ok(address);
        }

        if let Ok(string) = serde_wasm_bindgen::from_value::<String>(js_value.to_owned()) {
            Ok(Address::from_string(&string)?)
        } else if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(js_value.to_owned()) {
            Ok(Address::from(nimiq_keys::Address::deserialize_from_vec(
                &bytes,
            )?))
        } else {
            Err(JsError::new("Could not parse address"))
        }
    }

    /// Parses an address from a string representation, either user-friendly or hex format.
    ///
    /// Throws when an address cannot be parsed from the string.
    #[wasm_bindgen(js_name = fromString)]
    pub fn from_string(str: &str) -> Result<Address, JsError> {
        Ok(Address::from(nimiq_keys::Address::from_str(str)?))
    }

    /// Parses an address from its user-friendly string representation.
    ///
    /// Throws when an address cannot be parsed from the string.
    #[cfg(feature = "primitives")]
    #[wasm_bindgen(js_name = fromUserFriendlyAddress)]
    pub fn from_user_friendly_address(str: &str) -> Result<Address, JsError> {
        Ok(Address::from(
            nimiq_keys::Address::from_user_friendly_address(str)?,
        ))
    }

    /// Formats the address into a plain string format.
    #[wasm_bindgen(js_name = toPlain)]
    pub fn to_plain(&self) -> String {
        self.inner.to_user_friendly_address()
    }

    /// Formats the address into user-friendly IBAN format.
    #[cfg(feature = "primitives")]
    #[wasm_bindgen(js_name = toUserFriendlyAddress)]
    pub fn to_user_friendly_address(&self) -> String {
        self.inner.to_user_friendly_address()
    }

    /// Formats the address into hex format.
    #[cfg(feature = "primitives")]
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        self.inner.to_hex()
    }

    #[cfg(feature = "primitives")]
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }
}

impl From<nimiq_keys::Address> for Address {
    fn from(address: nimiq_keys::Address) -> Self {
        Address { inner: address }
    }
}

impl Address {
    pub fn native_ref(&self) -> &nimiq_keys::Address {
        &self.inner
    }

    #[cfg(feature = "client")]
    pub fn native(&self) -> nimiq_keys::Address {
        self.inner.clone()
    }

    #[cfg(feature = "client")]
    pub fn take_native(self) -> nimiq_keys::Address {
        self.inner
    }
}

#[cfg(feature = "primitives")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Address | string | Uint8Array")]
    pub type AddressAnyType;

    #[wasm_bindgen(typescript_type = "(Address | string | Uint8Array)[]")]
    pub type AddressAnyArrayType;
}

#[cfg(not(feature = "primitives"))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "string | Uint8Array")]
    pub type AddressAnyType;

    #[wasm_bindgen(typescript_type = "(string | Uint8Array)[]")]
    pub type AddressAnyArrayType;
}
