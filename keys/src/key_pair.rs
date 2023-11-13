use ed25519_zebra::{SigningKey, VerificationKeyBytes};
use nimiq_utils::key_rng::SecureGenerate;
use rand_core::{CryptoRng, RngCore};
#[cfg(feature = "serde-derive")]
use serde::{Deserialize, Serialize};

use crate::{EdDSAPublicKey, PrivateKey, Signature};

#[derive(Default, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde-derive", derive(Serialize, Deserialize))]
pub struct KeyPair {
    pub public: EdDSAPublicKey,
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
        let pub_key = EdDSAPublicKey(VerificationKeyBytes::from(&zebra_priv_key));
        KeyPair {
            private: priv_key,
            public: pub_key,
        }
    }
}

impl From<PrivateKey> for KeyPair {
    fn from(private_key: PrivateKey) -> Self {
        KeyPair {
            public: EdDSAPublicKey::from(&private_key),
            private: private_key,
        }
    }
}
