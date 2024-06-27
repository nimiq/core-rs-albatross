use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::{address::Address, primitives::private_key::PrivateKey};

/// The secret (private) part of an asymmetric key pair that is typically used to digitally sign or decrypt data.
#[wasm_bindgen]
pub struct ExtendedPrivateKey {
    inner: nimiq_key_derivation::ExtendedPrivateKey,
}

#[wasm_bindgen]
impl ExtendedPrivateKey {
    /// Deserializes an extended private key from a byte array.
    ///
    /// Throws when the byte array contains less than 64 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<ExtendedPrivateKey, JsError> {
        Ok(ExtendedPrivateKey::from(
            nimiq_key_derivation::ExtendedPrivateKey::deserialize_from_vec(bytes)?,
        ))
    }

    #[wasm_bindgen(js_name = generateMasterKey)]
    pub fn generate_master_key(seed: &[u8]) -> ExtendedPrivateKey {
        ExtendedPrivateKey::from(nimiq_key_derivation::ExtendedPrivateKey::from_seed(
            seed.to_vec(),
        ))
    }

    #[wasm_bindgen(js_name = isValidPath)]
    pub fn is_valid_path(path: String) -> bool {
        nimiq_key_derivation::ExtendedPrivateKey::is_valid_path(&path)
    }

    #[wasm_bindgen(js_name = derivePathFromSeed)]
    pub fn derive_path_from_seed(path: String, seed: &[u8]) -> Result<ExtendedPrivateKey, JsError> {
        let key = nimiq_key_derivation::ExtendedPrivateKey::from_seed(seed.to_vec());
        Ok(ExtendedPrivateKey::from(
            key.derive_path(&path).ok_or(JsError::new("Invalid path"))?,
        ))
    }

    /// Creates a new extended private key from a private key and chain code.
    ///
    /// Throws when the chain code is not exactly 32 bytes long.
    #[wasm_bindgen(constructor)]
    pub fn new(key: &PrivateKey, chain_code: &[u8]) -> Result<ExtendedPrivateKey, JsError> {
        if chain_code.len() != nimiq_key_derivation::ExtendedPrivateKey::CHAIN_CODE_SIZE {
            return Err(JsError::new(
                "Extended private key primitive: Invalid chain code length",
            ));
        }
        Ok(ExtendedPrivateKey::from(
            nimiq_key_derivation::ExtendedPrivateKey::new_unchecked(
                key.native_ref().clone(),
                chain_code.try_into().unwrap(),
            ),
        ))
    }

    pub fn derive(&self, index: u32) -> Result<ExtendedPrivateKey, JsError> {
        Ok(ExtendedPrivateKey::from(
            self.inner
                .derive(index)
                .ok_or(JsError::new("Invalid index"))?,
        ))
    }

    #[wasm_bindgen(js_name = derivePath)]
    pub fn derive_path(&self, path: String) -> Result<ExtendedPrivateKey, JsError> {
        Ok(ExtendedPrivateKey::from(
            self.inner
                .derive_path(&path)
                .ok_or(JsError::new("Invalid path"))?,
        ))
    }

    /// Serializes the extended private key to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Parses an extended private key from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 64 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<ExtendedPrivateKey, JsError> {
        ExtendedPrivateKey::unserialize(hex::decode(hex)?.as_slice())
    }

    /// Formats the extended private key into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }

    #[wasm_bindgen(getter, js_name = privateKey)]
    pub fn private_key(&self) -> PrivateKey {
        PrivateKey::from(self.inner.to_private_key())
    }

    #[wasm_bindgen(js_name = toAddress)]
    pub fn to_address(&self) -> Address {
        Address::from(self.inner.to_address())
    }
}

impl From<nimiq_key_derivation::ExtendedPrivateKey> for ExtendedPrivateKey {
    fn from(extended_private_key: nimiq_key_derivation::ExtendedPrivateKey) -> ExtendedPrivateKey {
        ExtendedPrivateKey {
            inner: extended_private_key,
        }
    }
}

impl ExtendedPrivateKey {
    pub fn native_ref(&self) -> &nimiq_key_derivation::ExtendedPrivateKey {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use crate::primitives::entropy::Entropy;

    const ENTROPY: &str = "fc0a0c62a4cc79211e58c1cd788d123d7af859668281ec2ec8861bd9a966d6bf";

    #[wasm_bindgen_test]
    pub fn it_can_derive_hd_wallets() {
        let entropy = Entropy::from_hex(ENTROPY).map_err(JsValue::from).unwrap();
        let ext_priv_key = entropy
            .to_extended_private_key(None)
            .map_err(JsValue::from)
            .unwrap();
        let derived_key = ext_priv_key
            .derive_path("m/44'/242'/0'/0'".to_string())
            .map_err(JsValue::from)
            .unwrap();
        let address = derived_key.to_address().to_user_friendly_address();
        assert_eq!(address, "NQ54 7EA0 SGCF 28VB L9H6 9KYG PG2U ATS3 CSN5")
    }
}
