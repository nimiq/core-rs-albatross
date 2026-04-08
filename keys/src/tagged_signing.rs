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
        let Ok(signature) = Ed25519Signature::from_bytes(sig) else {
            return false;
        };

        self.verify(&signature, msg)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;

    use super::*;
    use crate::SecureGenerate;

    #[test]
    fn tagged_verify_rejects_invalid_signature_lengths() {
        let keypair = KeyPair::generate(&mut test_rng(false));
        let message = b"test message";
        let signature = keypair.sign(message);

        assert!(TaggedPublicKey::verify(
            &keypair.public,
            message,
            &signature.to_bytes(),
        ));
        assert!(!TaggedPublicKey::verify(
            &keypair.public,
            message,
            &[0u8; Ed25519Signature::SIZE - 1],
        ));
        assert!(!TaggedPublicKey::verify(
            &keypair.public,
            message,
            &[0u8; Ed25519Signature::SIZE + 1],
        ));
    }
}
