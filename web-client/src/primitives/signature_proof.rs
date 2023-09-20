use nimiq_serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::{
    address::Address,
    primitives::{
        public_key::PublicKey, signature::Signature, webauthn_public_key::WebauthnPublicKey,
    },
};

/// A signature proof represents a signature together with its public key and the public key's merkle path.
/// It is used as the proof for transactions.
#[wasm_bindgen]
pub struct SignatureProof {
    inner: nimiq_transaction::SignatureProof,
}

#[wasm_bindgen]
impl SignatureProof {
    /// Creates a EdDSA/Schnorr signature proof for a single-sig signature.
    #[wasm_bindgen(js_name = singleSig)]
    pub fn single_sig(public_key: &PublicKey, signature: &Signature) -> SignatureProof {
        SignatureProof::from_native(nimiq_transaction::SignatureProof::EdDSA(
            nimiq_transaction::EdDSASignatureProof::from(
                *public_key.native_ref(),
                signature.native_ref().clone(),
            ),
        ))
    }

    /// Creates a ECDSA/Webauthn signature proof for a single-sig signature.
    #[wasm_bindgen(js_name = webauthnSingleSig)]
    pub fn webauthn_single_sig(
        public_key: &WebauthnPublicKey,
        signature: &Signature,
        host: String,
        authenticator_data: &[u8],
        // TODO: Improve DX by taking the raw clientDataJSON bytes and converting them to what we need internally.
        client_data_flags: Option<u8>,
        client_data_extra_fields: Option<String>,
    ) -> SignatureProof {
        SignatureProof::from_native(nimiq_transaction::SignatureProof::ECDSA(
            nimiq_transaction::WebauthnSignatureProof::from(
                *public_key.native_ref(),
                signature.native_ref().clone(),
                host.to_string(),
                // TODO: Calculate RP ID from `host` and verify it against the first 32 bytes of `authenticator_data`.
                authenticator_data[32..].to_vec(),
                nimiq_transaction::WebauthnClientDataFlags::from_bits_truncate(
                    client_data_flags.unwrap_or(0),
                ),
                client_data_extra_fields.unwrap_or("".to_string()),
            ),
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
        match self.inner {
            nimiq_transaction::SignatureProof::EdDSA(ref signature_proof) => {
                Signature::from_native(signature_proof.signature.clone())
            }
            nimiq_transaction::SignatureProof::ECDSA(ref signature_proof) => {
                Signature::from_native(signature_proof.signature.clone())
            }
        }
    }

    /// The embedded public key.
    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> PublicKeyUnion {
        match self.inner {
            nimiq_transaction::SignatureProof::EdDSA(ref signature_proof) => {
                let key = PublicKey::from_native(signature_proof.public_key);
                JsValue::unchecked_into(key.into())
            }
            nimiq_transaction::SignatureProof::ECDSA(ref signature_proof) => {
                let key = WebauthnPublicKey::from_native(signature_proof.public_key);
                JsValue::unchecked_into(key.into())
            }
        }
    }

    /// Serializes the proof to a byte array for extended transactions, e.g. for assigning it to a `transaction.proof` field.
    #[wasm_bindgen(js_name = serializeExtended)]
    pub fn serialize_extended(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Serializes the proof to a byte array for basic transactions, e.g. for assigning it to a `transaction.proof` field.
    #[wasm_bindgen(js_name = serializeBasic)]
    pub fn serialize_basic(&self) -> Result<Vec<u8>, JsError> {
        match self.inner {
            nimiq_transaction::SignatureProof::EdDSA(ref signature_proof) => {
                Ok(signature_proof.serialize_to_vec())
            }
            _ => Err(JsError::new(
                "Unsupported proof type for basic transactions",
            )),
        }
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

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PublicKey | WebauthnPublicKey")]
    pub type PublicKeyUnion;
}
