use js_sys::Array;
use nimiq_serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::{
    address::Address,
    primitives::{
        es256_public_key::ES256PublicKey, es256_signature::ES256Signature, public_key::PublicKey,
        signature::Signature,
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
    /// Creates a Ed25519/Schnorr signature proof for a single-sig signature.
    #[wasm_bindgen(js_name = singleSig)]
    pub fn single_sig(public_key: &PublicKey, signature: &Signature) -> SignatureProof {
        SignatureProof::from(nimiq_transaction::SignatureProof::from_ed25519(
            *public_key.native_ref(),
            signature.native_ref().clone(),
        ))
    }

    /// Creates a Ed25519/Schnorr signature proof for a multi-sig signature.
    /// The public keys can also include ES256 keys.
    #[wasm_bindgen(js_name = multiSig)]
    pub fn multi_sig(
        signer_key: &PublicKey,
        public_keys: &PublicKeyUnionArray,
        signature: &Signature,
    ) -> Result<SignatureProof, JsError> {
        let mut public_keys: Vec<_> = SignatureProof::unpack_public_keys(public_keys)?
            .into_iter()
            .map(|k| match k {
                nimiq_keys::PublicKey::Ed25519(ref public_key) => public_key.serialize_to_vec(),
                nimiq_keys::PublicKey::ES256(ref public_key) => public_key.serialize_to_vec(),
            })
            .collect();
        public_keys.sort_unstable();

        let merkle_path = nimiq_utils::merkle::Blake2bMerklePath::new::<
            nimiq_hash::Blake2bHasher,
            Vec<u8>,
        >(&public_keys, &signer_key.native_ref().serialize_to_vec());

        Ok(SignatureProof::from(nimiq_transaction::SignatureProof {
            public_key: nimiq_keys::PublicKey::Ed25519(*signer_key.native_ref()),
            merkle_path,
            signature: nimiq_keys::Signature::Ed25519(signature.native_ref().clone()),
            webauthn_fields: None,
        }))
    }

    /// Creates a Webauthn signature proof for a single-sig signature.
    #[wasm_bindgen(js_name = webauthnSingleSig)]
    pub fn webauthn_single_sig(
        public_key: &PublicKeyUnion,
        signature: &SignatureUnion,
        authenticator_data: &[u8],
        client_data_json: &str,
    ) -> Result<SignatureProof, JsError> {
        let public_key = SignatureProof::unpack_public_key(public_key)?;
        let signature = SignatureProof::unpack_signature(signature)?;

        Ok(SignatureProof::from(
            nimiq_transaction::SignatureProof::try_from_webauthn(
                public_key,
                None,
                signature,
                authenticator_data,
                client_data_json,
            )?,
        ))
    }

    /// Creates a Webauthn signature proof for a multi-sig signature.
    #[wasm_bindgen(js_name = webauthnMultiSig)]
    pub fn webauthn_multi_sig(
        signer_key: &PublicKeyUnion,
        public_keys: &PublicKeyUnionArray,
        signature: &SignatureUnion,
        authenticator_data: &[u8],
        client_data_json: &[u8],
    ) -> Result<SignatureProof, JsError> {
        let signer_key = SignatureProof::unpack_public_key(signer_key)?;
        let signature = SignatureProof::unpack_signature(signature)?;

        let mut public_keys: Vec<_> = SignatureProof::unpack_public_keys(public_keys)?
            .into_iter()
            .map(|k| match k {
                nimiq_keys::PublicKey::Ed25519(ref public_key) => public_key.serialize_to_vec(),
                nimiq_keys::PublicKey::ES256(ref public_key) => public_key.serialize_to_vec(),
            })
            .collect();
        public_keys.sort_unstable();

        let merkle_path = nimiq_utils::merkle::Blake2bMerklePath::new::<
            nimiq_hash::Blake2bHasher,
            Vec<u8>,
        >(&public_keys, &signer_key.serialize_to_vec());

        Ok(SignatureProof::from(
            nimiq_transaction::SignatureProof::try_from_webauthn(
                signer_key,
                Some(merkle_path),
                signature,
                authenticator_data,
                client_data_json,
            )?,
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
    pub fn signature(&self) -> SignatureUnion {
        match self.inner.signature {
            nimiq_keys::Signature::Ed25519(ref signature) => {
                let signature = Signature::from(signature.clone());
                JsValue::unchecked_into(signature.into())
            }
            nimiq_keys::Signature::ES256(ref signature) => {
                let signature = ES256Signature::from(signature.clone());
                JsValue::unchecked_into(signature.into())
            }
        }
    }

    /// The embedded public key.
    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> PublicKeyUnion {
        match self.inner.public_key {
            nimiq_keys::PublicKey::Ed25519(ref public_key) => {
                let key = PublicKey::from(*public_key);
                JsValue::unchecked_into(key.into())
            }
            nimiq_keys::PublicKey::ES256(ref public_key) => {
                let key = ES256PublicKey::from(*public_key);
                JsValue::unchecked_into(key.into())
            }
        }
    }

    /// Serializes the proof to a byte array, e.g. for assigning it to a `transaction.proof` field.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }
}

impl From<nimiq_transaction::SignatureProof> for SignatureProof {
    fn from(signature_proof: nimiq_transaction::SignatureProof) -> Self {
        SignatureProof {
            inner: signature_proof,
        }
    }
}

impl SignatureProof {
    pub fn native_ref(&self) -> &nimiq_transaction::SignatureProof {
        &self.inner
    }

    fn unpack_public_keys(
        public_keys: &PublicKeyUnionArray,
    ) -> Result<Vec<nimiq_keys::PublicKey>, JsError> {
        // Unpack the array of public keys
        let js_value: &JsValue = public_keys.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`public_keys` must be an array"))?;

        if array.length() == 0 {
            return Err(JsError::new("No public keys provided"));
        }

        let mut public_keys = Vec::<_>::with_capacity(array.length().try_into()?);
        for item in array.iter() {
            let public_key = SignatureProof::unpack_public_key(item.unchecked_ref())
                .map_err(|_| JsError::new("Invalid public key in array"))?;
            public_keys.push(public_key);
        }

        Ok(public_keys)
    }

    fn unpack_public_key(public_key: &PublicKeyUnion) -> Result<nimiq_keys::PublicKey, JsError> {
        let js_value: &JsValue = public_key.unchecked_ref();
        let public_key = if let Ok(key) = PublicKey::try_from(js_value) {
            nimiq_keys::PublicKey::Ed25519(*key.native_ref())
        } else if let Ok(key) = ES256PublicKey::try_from(js_value) {
            nimiq_keys::PublicKey::ES256(*key.native_ref())
        } else {
            return Err(JsError::new("Invalid public key"));
        };

        Ok(public_key)
    }

    fn unpack_signature(signature: &SignatureUnion) -> Result<nimiq_keys::Signature, JsError> {
        let js_value: &JsValue = signature.unchecked_ref();
        let signature = if let Ok(sig) = Signature::try_from(js_value) {
            nimiq_keys::Signature::Ed25519(sig.native_ref().clone())
        } else if let Ok(sig) = ES256Signature::try_from(js_value) {
            nimiq_keys::Signature::ES256(sig.native_ref().clone())
        } else {
            return Err(JsError::new("Invalid signature"));
        };

        Ok(signature)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PublicKey | ES256PublicKey")]
    pub type PublicKeyUnion;

    #[wasm_bindgen(typescript_type = "(PublicKey | ES256PublicKey)[]")]
    pub type PublicKeyUnionArray;

    #[wasm_bindgen(typescript_type = "Signature | ES256Signature")]
    pub type SignatureUnion;
}

#[cfg(test)]
mod tests {
    use wasm_bindgen::prelude::JsValue;
    use wasm_bindgen_test::*;

    use crate::primitives::{
        es256_public_key::ES256PublicKey, es256_signature::ES256Signature,
        signature_proof::SignatureProof,
    };

    /// Tests a signature generated with Desktop Chrome, which follows the Webauthn standard.
    #[wasm_bindgen_test]
    fn it_can_create_a_standard_webauthn_signature_proof() {
        let public_key = ES256PublicKey::new(
            &hex::decode("02af70ae2e8232eb5ca2f8a4c47a71d9cd6ea62f552467f0d3c56428be47d63808")
                .unwrap(),
        )
        .map_err(JsValue::from)
        .unwrap();
        let signature = ES256Signature::from_asn1(&hex::decode("3045022100c9cbe35dc11f9eb1ca4ea6ed0b63f7492f60287644adde1b1f833cb5db93b03f0220664b8bc653e7dcbfc4add0d38c65af624f1787102b5a64155c0d2521a924fd49").unwrap())
        .map_err(JsValue::from)
        .unwrap();
        let authenticator_data = hex::decode(
            "49960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d97630100000007",
        )
        .unwrap();
        let client_data_json = r#"{"type":"webauthn.get","challenge":"u7CRZnGHrWoR9ix5llhl2VopsexuW36FKszgHBx4FfM","origin":"http://localhost:5173","crossOrigin":false}"#;
        let proof = SignatureProof::webauthn_single_sig(
            &JsValue::from(public_key).into(),
            &JsValue::from(signature).into(),
            &authenticator_data,
            client_data_json,
        );
        assert!(proof.is_ok());
        // if proof.is_err() {
        //     console_log!("{:?}", proof.map_err(JsValue::from).err().unwrap());
        // }
    }

    /// Tests a signature generated with Android Chrome, which has no crossOrigin field in the client data JSON
    /// and has escaped forward slashes in the origin. It also has extra fields in the client data JSON.
    #[wasm_bindgen_test]
    fn it_can_create_a_nonstandard_webauthn_signature_proof() {
        let public_key = ES256PublicKey::new(
            &hex::decode("0327e1f7995bde5df8a22bd9c27833b532d79c2350e61fc9a85621d1438eabeb7c")
                .unwrap(),
        )
        .map_err(JsValue::from)
        .unwrap();
        let signature = ES256Signature::from_asn1(&hex::decode("30440220715d1485dd0eb8f87afc2de77a75f90d5119c19e42090419bd2a54d334ffb91002204bf51841ec7872d42268696d5074fc2635ac4cedb72e90b3f18a053debacb2fc").unwrap())
        .map_err(JsValue::from)
        .unwrap();
        let authenticator_data = hex::decode(
            "7a03a16dfe0c4358b79eebe4f25cba56ec7aa7c8331f46a96988006db440e690050000003c",
        )
        .unwrap();
        let client_data_json = r#"{"type":"webauthn.get","challenge":"OCAjaXEXs-P4zqEk-61MhWPOq9a8VpDL2qf3ZwnAI9I","origin":"https:\/\/webauthn.pos.nimiqwatch.com","androidPackageName":"com.android.chrome"}"#;
        let proof = SignatureProof::webauthn_single_sig(
            &JsValue::from(public_key).into(),
            &JsValue::from(signature).into(),
            &authenticator_data,
            client_data_json,
        );
        assert!(proof.is_ok());
        // if proof.is_err() {
        //     console_log!("{:?}", proof.map_err(JsValue::from).err().unwrap());
        // }
    }
}
