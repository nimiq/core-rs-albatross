use ed25519_dalek;

use beserial::{Deserialize, Serialize};
use utils::key_rng::{CryptoRng, Rng, SecureGenerate};

use crate::{PrivateKey, PublicKey, Signature};

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KeyPair {
    pub public: PublicKey,
    pub private: PrivateKey,
}

impl KeyPair {
    pub fn sign(&self, data: &[u8]) -> Signature {
        let ext_signature =
            ed25519_dalek::ExpandedSecretKey::from(&self.private.0).sign(data, &self.public.0);
        Signature(ext_signature)
    }
}

impl SecureGenerate for KeyPair {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self {
        let key_pair = ed25519_dalek::Keypair::generate(rng);
        let priv_key = PrivateKey(key_pair.secret);
        let pub_key = PublicKey(key_pair.public);
        KeyPair {
            private: priv_key,
            public: pub_key,
        }
    }
}

impl From<PrivateKey> for KeyPair {
    fn from(private_key: PrivateKey) -> Self {
        KeyPair {
            public: PublicKey::from(&private_key),
            private: private_key,
        }
    }
}
