use nimiq_mnemonic::key_derivation::ToExtendedPrivateKey;
use wasm_bindgen::prelude::*;

use crate::primitives::{entropy::Entropy, extended_private_key::ExtendedPrivateKey};

#[wasm_bindgen]
pub struct MnemonicUtils;

#[wasm_bindgen]
impl MnemonicUtils {
    /// Converts an Entropy to a mnemonic.
    #[wasm_bindgen(js_name = entropyToMnemonic)]
    pub fn entropy_to_mnemonic(entropy: Entropy) -> Vec<String> {
        entropy.to_mnemonic()
    }

    /// Converts a mnemonic to an Entropy.
    #[wasm_bindgen(js_name = mnemonicToEntropy)]
    pub fn mnemonic_to_entropy(mnemonic: Vec<String>) -> Result<Entropy, JsError> {
        let mnemonic = nimiq_mnemonic::Mnemonic::from_words_unchecked(mnemonic);
        if let Some(entropy) = mnemonic.to_entropy(nimiq_mnemonic::WORDLIST_EN) {
            Ok(Entropy::from(entropy))
        } else {
            Err(JsError::new("Invalid mnemonic"))
        }
    }

    /// Converts a mnemonic to a seed.
    ///
    /// Optionally takes a password to use for the seed derivation.
    #[wasm_bindgen(js_name = mnemonicToSeed)]
    pub fn mnemonic_to_seed(
        mnemonic: Vec<String>,
        password: Option<String>,
    ) -> Result<Vec<u8>, JsError> {
        let mnemonic = nimiq_mnemonic::Mnemonic::from_words_unchecked(mnemonic);
        mnemonic
            .to_seed(password.as_deref())
            .map_err(|_| JsError::new("Invalid mnemonic"))
    }

    /// Converts a mnemonic to an extended private key.
    ///
    /// Optionally takes a password to use for the seed derivation.
    #[wasm_bindgen(js_name = mnemonicToExtendedPrivateKey)]
    pub fn mnemonic_to_extended_private_key(
        mnemonic: Vec<String>,
        password: Option<String>,
    ) -> Result<ExtendedPrivateKey, JsError> {
        let mnemonic = nimiq_mnemonic::Mnemonic::from_words_unchecked(mnemonic);
        let ext_priv_key = mnemonic
            .to_master_key(password.as_deref())
            .map_err(|_| JsError::new("Invalid mnemonic"))?;
        Ok(ExtendedPrivateKey::from(ext_priv_key))
    }

    /// Tests if a mnemonic can be both for a legacy Nimiq wallet and a BIP39 wallet.
    #[wasm_bindgen(js_name = isCollidingChecksum)]
    pub fn is_colliding_checksum(entropy: Entropy) -> bool {
        entropy.native_ref().is_colliding_checksum()
    }

    /// Gets the type of a mnemonic.
    ///
    /// Return values:
    /// - `MnemonicType.LEGACY` => the mnemonic is for a legacy Nimiq wallet.
    /// - `MnemonicType.BIP39` => the mnemonic is for a BIP39 wallet.
    /// - `MnemonicType.UNKNOWN` => the mnemonic can be for both.
    /// - `MnemonicType.INVALID` => the mnemonic is invalid.
    #[wasm_bindgen(js_name = getMnemonicType)]
    pub fn get_mnemonic_type(mnemonic: Vec<String>) -> MnemonicType {
        let mnemonic = nimiq_mnemonic::Mnemonic::from_words_unchecked(mnemonic);
        JsValue::from(mnemonic.get_type(nimiq_mnemonic::WORDLIST_EN) as i8).unchecked_into()
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "MnemonicType")]
    pub type MnemonicType;
}

#[wasm_bindgen(typescript_custom_section)]
const TS_MNEMONIC_ENUM: &'static str = r#"export enum MnemonicType {
    INVALID = -2,
    UNKNOWN = -1,
    LEGACY = 0,
    BIP39 = 1,
}"#;

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::MnemonicUtils;
    use crate::primitives::entropy::Entropy;

    const ENTROPY: &str = "fc0a0c62a4cc79211e58c1cd788d123d7af859668281ec2ec8861bd9a966d6bf";

    #[wasm_bindgen_test]
    pub fn it_can_convert_between_entropy_and_mnemonic() {
        let entropy = Entropy::from_hex(ENTROPY).map_err(JsValue::from).unwrap();
        let mnemonic = MnemonicUtils::entropy_to_mnemonic(entropy);
        assert_eq!(
            mnemonic,
            vec![
                "winter", "expire", "board", "end", "shy", "mountain", "just", "blouse", "sniff",
                "settle", "duty", "kit", "question", "coach", "old", "expand", "umbrella", "iron",
                "canoe", "dash", "once", "recall", "foot", "width"
            ]
        );
        let entropy = MnemonicUtils::mnemonic_to_entropy(mnemonic)
            .map_err(JsValue::from)
            .unwrap();
        assert_eq!(entropy.to_hex(), ENTROPY);
    }
}
