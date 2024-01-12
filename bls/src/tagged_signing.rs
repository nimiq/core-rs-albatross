#[cfg(feature = "serde-derive")]
use nimiq_serde::Deserialize;
use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedPublicKey};

use crate::{CompressedSignature, KeyPair, PublicKey};

impl TaggedKeyPair for KeyPair {
    type PublicKey = PublicKey;

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.sign(&message).compress().as_ref().to_vec()
    }
}

#[cfg(feature = "serde-derive")]
impl TaggedPublicKey for PublicKey {
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        let signature = match CompressedSignature::deserialize_from_vec(sig) {
            Ok(compressed_signature) => match compressed_signature.uncompress() {
                Ok(signature) => signature,
                Err(_) => return false,
            },
            Err(_) => return false,
        };
        self.verify(&msg, &signature)
    }
}
