use ed25519_zebra::{SigningKey, VerificationKeyBytes};
use nimiq_utils::key_rng::SecureGenerate;
use rand_core::{CryptoRng, RngCore};
#[cfg(feature = "serde-derive")]
use serde::{Deserialize, Serialize};

use crate::{Ed25519PublicKey, Ed25519Signature, PrivateKey};

#[derive(Default, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde-derive", derive(Serialize, Deserialize))]
pub struct KeyPair {
    pub private: PrivateKey,
    pub public: Ed25519PublicKey,
}

impl KeyPair {
    pub fn sign(&self, data: &[u8]) -> Ed25519Signature {
        let ext_signature = self.private.0.sign(data);
        Ed25519Signature(ext_signature)
    }
}

impl SecureGenerate for KeyPair {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let zebra_priv_key = SigningKey::new(rng);
        let priv_key = PrivateKey(zebra_priv_key);
        let pub_key = Ed25519PublicKey(VerificationKeyBytes::from(&zebra_priv_key));
        KeyPair {
            private: priv_key,
            public: pub_key,
        }
    }
}

impl From<PrivateKey> for KeyPair {
    fn from(private_key: PrivateKey) -> Self {
        let public_key = Ed25519PublicKey::from(&private_key);
        KeyPair {
            private: private_key,
            public: public_key,
        }
    }
}
