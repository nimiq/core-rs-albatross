use wasm_bindgen::prelude::*;

use beserial::Serialize;

use crate::address::Address;
use crate::public_key::PublicKey;
use crate::signature::Signature;

#[wasm_bindgen]
pub struct SignatureProof {
    inner: nimiq_transaction::SignatureProof,
}

#[wasm_bindgen]
impl SignatureProof {
    #[wasm_bindgen(js_name = singleSig)]
    pub fn single_sig(public_key: &PublicKey, signature: &Signature) -> SignatureProof {
        SignatureProof::from_native(nimiq_transaction::SignatureProof::from(
            *public_key.native_ref(),
            signature.native_ref().clone(),
        ))
    }

    pub fn verify(&self, data: &[u8]) -> bool {
        self.inner.verify(data)
    }

    #[wasm_bindgen(js_name = isSignedBy)]
    pub fn is_signed_by(&self, sender: &Address) -> bool {
        self.inner.is_signed_by(sender.native_ref())
    }

    #[wasm_bindgen(getter)]
    pub fn signature(&self) -> Signature {
        Signature::from_native(self.inner.signature.clone())
    }

    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> PublicKey {
        PublicKey::from_native(self.inner.public_key)
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }
}

impl SignatureProof {
    pub fn from_native(signature_proof: nimiq_transaction::SignatureProof) -> SignatureProof {
        SignatureProof {
            inner: signature_proof,
        }
    }

    pub fn native_ref(&self) -> &nimiq_transaction::SignatureProof {
        &self.inner
    }
}
