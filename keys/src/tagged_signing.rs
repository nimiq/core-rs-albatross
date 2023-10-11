use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedPublicKey};

use crate::{KeyPair, EdDSAPublicKey, Signature};

impl TaggedKeyPair for KeyPair {
    type PublicKey = EdDSAPublicKey;

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.sign(message).to_bytes().to_vec()
    }
}

impl TaggedPublicKey for EdDSAPublicKey {
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        self.verify(&Signature::from_bytes(sig).unwrap(), msg)
    }
}
