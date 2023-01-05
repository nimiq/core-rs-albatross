use ed25519_zebra::{SigningKey, VerificationKeyBytes};
use rand_core::{CryptoRng, RngCore};

use beserial::{Deserialize, Serialize};
use nimiq_utils::key_rng::SecureGenerate;

use crate::{PrivateKey, PublicKey, Signature};

#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct KeyPair {
    pub public: PublicKey,
    pub private: PrivateKey,
}

impl KeyPair {
    pub fn sign(&self, data: &[u8]) -> Signature {
        let ext_signature = self.private.0.sign(data);
        Signature(ext_signature)
    }
}

impl SecureGenerate for KeyPair {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let zebra_priv_key = SigningKey::new(rng);
        let priv_key = PrivateKey(zebra_priv_key);
        let pub_key = PublicKey(VerificationKeyBytes::from(&zebra_priv_key));
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
