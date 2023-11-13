use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedPublicKey};

use crate::{Ed25519PublicKey, Ed25519Signature, KeyPair};

impl TaggedKeyPair for KeyPair {
    type PublicKey = Ed25519PublicKey;

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.sign(message).to_bytes().to_vec()
    }
}

impl TaggedPublicKey for Ed25519PublicKey {
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        self.verify(&Ed25519Signature::from_bytes(sig).unwrap(), msg)
    }
}
