use ed25519_dalek;
use rand::OsRng;
use sha2;

use crate::consensus::base::primitive::crypto::{PublicKey, PrivateKey, Signature};

pub struct KeyPair {
    pub public: PublicKey,
    pub private: PrivateKey,
}

impl KeyPair {
    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        let key_pair = ed25519_dalek::Keypair::generate::<sha2::Sha512>(&mut cspring);
        let priv_key = PrivateKey(key_pair.secret);
        let pub_key = PublicKey(key_pair.public);
        return KeyPair { private: priv_key, public: pub_key };
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        let ext_signature = self.private.0.expand::<sha2::Sha512>().sign::<sha2::Sha512>(data, &self.public.0);
        return Signature(ext_signature);
    }
}

impl From<PrivateKey> for KeyPair {
    fn from(private_key: PrivateKey) -> Self {
        return KeyPair { public: PublicKey::from(&private_key), private: private_key };
    }
}
