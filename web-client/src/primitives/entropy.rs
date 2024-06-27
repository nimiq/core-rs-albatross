use std::str::FromStr;

use nimiq_keys::SecureGenerate;
use nimiq_mnemonic::key_derivation::ToExtendedPrivateKey;
use wasm_bindgen::prelude::*;

use crate::primitives::extended_private_key::ExtendedPrivateKey;

/// The Entropy object represents a secret for derivation of hierarchical deterministic wallets via a mnemonic.
#[derive(Debug)]
#[wasm_bindgen]
pub struct Entropy {
    inner: nimiq_mnemonic::Entropy,
}

#[wasm_bindgen]
impl Entropy {
    /// Generates a new Entropy object from secure randomness.
    pub fn generate() -> Entropy {
        let entropy = nimiq_mnemonic::Entropy::generate_default_csprng();
        Entropy::from(entropy)
    }

    /// Parses an Entropy object from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 32 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<Entropy, JsError> {
        let entropy = nimiq_mnemonic::Entropy::from_str(&hex[0..64])?;
        Ok(Entropy::from(entropy))
    }

    /// Deserializes an Entropy object from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn unserialize(bytes: &[u8]) -> Result<Entropy, JsError> {
        if bytes.len() < nimiq_mnemonic::Entropy::SIZE {
            return Err(JsError::new("Entropy primitive: Invalid length"));
        }
        let entropy = nimiq_mnemonic::Entropy::from(bytes);
        Ok(Entropy::from(entropy))
    }

    #[wasm_bindgen(js_name = fromEncrypted)]
    pub fn from_encrypted(buf: &[u8], key: &[u8]) -> Result<Entropy, JsError> {
        let entropy =
            nimiq_mnemonic::Entropy::from_encrypted(buf, key).map_err(|str| JsError::new(&str))?;
        Ok(Entropy::from(entropy))
    }

    /// Creates a new Entropy from a byte array.
    ///
    /// Throws when the byte array is not exactly 32 bytes long.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<Entropy, JsError> {
        if bytes.len() != nimiq_mnemonic::Entropy::SIZE {
            return Err(JsError::new("Entropy primitive: Invalid length"));
        }
        Self::unserialize(bytes)
    }

    /// Serializes the Entropy to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.0.to_vec()
    }

    /// Formats the Entropy into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }

    #[wasm_bindgen(js_name = toExtendedPrivateKey)]
    pub fn to_extended_private_key(
        &self,
        password: Option<String>,
    ) -> Result<ExtendedPrivateKey, JsError> {
        let mnemonic = self.inner.to_mnemonic(nimiq_mnemonic::WORDLIST_EN);
        let ext_priv_key = mnemonic
            .to_master_key(password.as_deref())
            .map_err(|_| JsError::new("Invalid mnemonic"))?;
        Ok(ExtendedPrivateKey::from(ext_priv_key))
    }

    #[wasm_bindgen(js_name = toMnemonic)]
    pub fn to_mnemonic(&self) -> Vec<String> {
        self.inner
            .to_mnemonic(nimiq_mnemonic::WORDLIST_EN)
            .as_words()
    }

    #[wasm_bindgen(js_name = exportEncrypted)]
    pub fn export_encrypted(&self, key: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner
            .export_encrypted(key)
            .map_err(|str| JsError::new(&str))
    }
}

impl From<nimiq_mnemonic::Entropy> for Entropy {
    fn from(entropy: nimiq_mnemonic::Entropy) -> Entropy {
        Entropy { inner: entropy }
    }
}

impl Entropy {
    pub fn native_ref(&self) -> &nimiq_mnemonic::Entropy {
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
    pub fn it_can_encrypt_and_decrypt_entropy() {
        // Test self-encryption and -decryption
        let entropy = Entropy::from_hex(ENTROPY).map_err(JsValue::from).unwrap();
        let encrypted = entropy
            .export_encrypted(b"albatross")
            .map_err(JsValue::from)
            .unwrap();
        let decrypted = Entropy::from_encrypted(&encrypted, b"albatross")
            .map_err(JsValue::from)
            .unwrap();
        assert_eq!(entropy.native_ref(), decrypted.native_ref());

        // Test decryption of a known encrypted PoW entropy
        let decrypted = Entropy::from_encrypted(
            &hex::decode("030811202b6ef916ac79d92ad3bc6dbbc19045c3e6193e822c14c8e2d63ac60c3735ae42fefe73725a5c9e49f07c2fdb58a8221b98ec4451").unwrap(),
            b"albatross",
        ).map_err(JsValue::from).unwrap();
        assert_eq!(decrypted.to_hex(), ENTROPY);

        // Test failing decryption with a wrong password
        let failed = Entropy::from_encrypted(
            &hex::decode("030811202b6ef916ac79d92ad3bc6dbbc19045c3e6193e822c14c8e2d63ac60c3735ae42fefe73725a5c9e49f07c2fdb58a8221b98ec4451").unwrap(),
            b"not the password",
        ).unwrap_err();
        assert!(format!("{:?}", JsValue::from(failed)).starts_with("JsValue(Error: Invalid key\n"));
    }
}
