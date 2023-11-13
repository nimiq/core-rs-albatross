use nimiq_serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::{
    address::Address,
    primitives::{public_key::PublicKey, signature::Signature},
};

/// A signature proof represents a signature together with its public key and the public key's merkle path.
/// It is used as the proof for transactions.
#[wasm_bindgen]
pub struct SignatureProof {
    inner: nimiq_transaction::EdDSASignatureProof,
}

#[wasm_bindgen]
impl SignatureProof {
    /// Creates a signature proof for a single-sig signature.
    #[wasm_bindgen(js_name = singleSig)]
    pub fn single_sig(public_key: &PublicKey, signature: &Signature) -> SignatureProof {
        SignatureProof::from_native(nimiq_transaction::EdDSASignatureProof::from(
            *public_key.native_ref(),
            signature.native_ref().clone(),
        ))
    }

    /// Verifies the signature proof against the provided data.
    pub fn verify(&self, data: &[u8]) -> bool {
        self.inner.verify(data)
    }

    /// Checks if the signature proof is signed by the provided address.
    #[wasm_bindgen(js_name = isSignedBy)]
    pub fn is_signed_by(&self, sender: &Address) -> bool {
        self.inner.is_signed_by(sender.native_ref())
    }

    /// The embedded signature.
    #[wasm_bindgen(getter)]
    pub fn signature(&self) -> Signature {
        Signature::from_native(self.inner.signature.clone())
    }

    /// The embedded public key.
    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> PublicKey {
        PublicKey::from_native(self.inner.public_key)
    }

    /// Serializes the proof to a byte array, e.g. for assigning it to a `transaction.proof` field.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }
}

impl SignatureProof {
    pub fn from_native(signature_proof: nimiq_transaction::EdDSASignatureProof) -> SignatureProof {
        SignatureProof {
            inner: signature_proof,
        }
    }

    pub fn native_ref(&self) -> &nimiq_transaction::EdDSASignatureProof {
        &self.inner
    }
}
