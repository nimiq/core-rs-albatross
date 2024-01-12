use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedPublicKey};

use crate::{KeyPair, PublicKey, Signature};

impl TaggedKeyPair for KeyPair {
    type PublicKey = PublicKey;

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.sign(message).to_bytes().to_vec()
    }
}

impl TaggedPublicKey for PublicKey {
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        self.verify(&Signature::from_bytes(sig).unwrap(), msg)
    }
}
