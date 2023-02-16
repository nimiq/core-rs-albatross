use wasm_bindgen::prelude::*;

use nimiq_keys::SecureGenerate;

use crate::private_key::PrivateKey;
use crate::public_key::PublicKey;
use crate::signature::Signature;

#[wasm_bindgen]
pub struct KeyPair {
    inner: nimiq_keys::KeyPair,
}

#[wasm_bindgen]
impl KeyPair {
    pub fn generate() -> KeyPair {
        let key_pair = nimiq_keys::KeyPair::generate_default_csprng();
        KeyPair::from_native(key_pair)
    }

    pub fn derive(private_key: &PrivateKey) -> KeyPair {
        let key_pair = nimiq_keys::KeyPair::from(private_key.native_ref().clone());
        KeyPair::from_native(key_pair)
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        Signature::from_native(self.inner.sign(data))
    }

    #[wasm_bindgen(getter, js_name = privateKey)]
    pub fn private_key(&self) -> PrivateKey {
        PrivateKey::from_native(self.inner.private.clone())
    }

    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> PublicKey {
        PublicKey::from_native(self.inner.public)
    }
}

impl KeyPair {
    pub fn from_native(key_pair: nimiq_keys::KeyPair) -> KeyPair {
        KeyPair { inner: key_pair }
    }

    pub fn native_ref(&self) -> &nimiq_keys::KeyPair {
        &self.inner
    }
}
