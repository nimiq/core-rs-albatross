use wasm_bindgen::prelude::*;

use nimiq_keys::SecureGenerate;

use crate::private_key::PrivateKey;
use crate::public_key::PublicKey;
use crate::signature::Signature;

#[wasm_bindgen]
pub struct KeyPair {
    #[wasm_bindgen(readonly, js_name = privateKey)]
    pub private_key: PrivateKey,
    #[wasm_bindgen(readonly, js_name = publicKey)]
    pub public_key: PublicKey,
}

#[wasm_bindgen]
impl KeyPair {
    pub fn generate() -> KeyPair {
        let key_pair = nimiq_keys::KeyPair::generate_default_csprng();
        KeyPair::from_native(key_pair)
    }

    pub fn derive(private_key: &PrivateKey) -> KeyPair {
        let key_pair = nimiq_keys::KeyPair::from(private_key.to_native());
        KeyPair::from_native(key_pair)
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        let key_pair = self.to_native();
        Signature::from_native(key_pair.sign(data))
    }
}

impl KeyPair {
    pub fn from_native(key_pair: nimiq_keys::KeyPair) -> KeyPair {
        KeyPair {
            private_key: PrivateKey::from_native(key_pair.private),
            public_key: PublicKey::from_native(key_pair.public),
        }
    }

    pub fn to_native(&self) -> nimiq_keys::KeyPair {
        nimiq_keys::KeyPair {
            private: self.private_key.to_native(),
            public: self.public_key.to_native(),
        }
    }
}
