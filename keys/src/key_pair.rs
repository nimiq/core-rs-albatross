use ed25519_dalek;
use rand::rngs::OsRng;

use beserial::{Deserialize, Serialize};

use crate::{PrivateKey, PublicKey, Signature};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyPair {
    pub public: PublicKey,
    pub private: PrivateKey,
}

impl KeyPair {
    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        let key_pair = ed25519_dalek::Keypair::generate(&mut cspring);
        let priv_key = PrivateKey(key_pair.secret);
        let pub_key = PublicKey(key_pair.public);
        KeyPair { private: priv_key, public: pub_key }
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        let ext_signature = ed25519_dalek::ExpandedSecretKey::from(&self.private.0).sign(data, &self.public.0);
        Signature(ext_signature)
    }
}

impl From<PrivateKey> for KeyPair {
    fn from(private_key: PrivateKey) -> Self {
        KeyPair { public: PublicKey::from(&private_key), private: private_key }
    }
}
