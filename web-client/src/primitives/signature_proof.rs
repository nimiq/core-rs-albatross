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

    /// Creates a ES256/Webauthn signature proof for a single-sig signature.
    #[wasm_bindgen(js_name = webauthnSingleSig)]
    pub fn webauthn_single_sig(
        public_key: &PublicKeyUnion,
        signature: &SignatureUnion,
        authenticator_data: &[u8],
        client_data_json: &[u8],
    ) -> Result<SignatureProof, JsError> {
        let js_value: &JsValue = public_key.unchecked_ref();
        let public_key = if let Ok(key) = PublicKey::try_from(js_value) {
            nimiq_keys::PublicKey::Ed25519(*key.native_ref())
        } else if let Ok(key) = ES256PublicKey::try_from(js_value) {
            nimiq_keys::PublicKey::ES256(*key.native_ref())
        } else {
            return Err(JsError::new("Invalid public key"));
        };

        let js_value: &JsValue = signature.unchecked_ref();
        let signature = if let Ok(sig) = Signature::try_from(js_value) {
            nimiq_keys::Signature::Ed25519(sig.native_ref().clone())
        } else if let Ok(sig) = ES256Signature::try_from(js_value) {
            nimiq_keys::Signature::ES256(sig.native_ref().clone())
        } else {
            return Err(JsError::new("Invalid signature"));
        };

        Ok(SignatureProof::from(
            nimiq_transaction::SignatureProof::try_from_webauthn(
                public_key,
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

    #[cfg(test)]
    pub fn webauthn_fields_flags(&self) -> u8 {
        self.inner
            .webauthn_fields
            .as_ref()
            .unwrap()
            .client_data_flags
            .bits()
    }

    #[cfg(test)]
    pub fn webauthn_extra_fields(&self) -> String {
        self.inner
            .webauthn_fields
            .as_ref()
            .unwrap()
            .client_data_extra_fields
            .clone()
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
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PublicKey | ES256PublicKey")]
    pub type PublicKeyUnion;

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
        let public_key = ES256PublicKey::new(&[
            2, 175, 112, 174, 46, 130, 50, 235, 92, 162, 248, 164, 196, 122, 113, 217, 205, 110,
            166, 47, 85, 36, 103, 240, 211, 197, 100, 40, 190, 71, 214, 56, 8,
        ])
        .map_err(JsValue::from)
        .unwrap();
        let signature = ES256Signature::from_asn1(&[
            48, 69, 2, 33, 0, 201, 203, 227, 93, 193, 31, 158, 177, 202, 78, 166, 237, 11, 99, 247,
            73, 47, 96, 40, 118, 68, 173, 222, 27, 31, 131, 60, 181, 219, 147, 176, 63, 2, 32, 102,
            75, 139, 198, 83, 231, 220, 191, 196, 173, 208, 211, 140, 101, 175, 98, 79, 23, 135,
            16, 43, 90, 100, 21, 92, 13, 37, 33, 169, 36, 253, 73,
        ])
        .map_err(JsValue::from)
        .unwrap();
        let authenticator_data = &[
            73, 150, 13, 229, 136, 14, 140, 104, 116, 52, 23, 15, 100, 118, 96, 91, 143, 228, 174,
            185, 162, 134, 50, 199, 153, 92, 243, 186, 131, 29, 151, 99, 1, 0, 0, 0, 7,
        ];
        let client_data_json = &[
            123, 34, 116, 121, 112, 101, 34, 58, 34, 119, 101, 98, 97, 117, 116, 104, 110, 46, 103,
            101, 116, 34, 44, 34, 99, 104, 97, 108, 108, 101, 110, 103, 101, 34, 58, 34, 117, 55,
            67, 82, 90, 110, 71, 72, 114, 87, 111, 82, 57, 105, 120, 53, 108, 108, 104, 108, 50,
            86, 111, 112, 115, 101, 120, 117, 87, 51, 54, 70, 75, 115, 122, 103, 72, 66, 120, 52,
            70, 102, 77, 34, 44, 34, 111, 114, 105, 103, 105, 110, 34, 58, 34, 104, 116, 116, 112,
            58, 47, 47, 108, 111, 99, 97, 108, 104, 111, 115, 116, 58, 53, 49, 55, 51, 34, 44, 34,
            99, 114, 111, 115, 115, 79, 114, 105, 103, 105, 110, 34, 58, 102, 97, 108, 115, 101,
            125,
        ];
        let proof = SignatureProof::webauthn_single_sig(
            &JsValue::from(public_key).into(),
            &JsValue::from(signature).into(),
            authenticator_data,
            client_data_json,
        );
        assert!(proof.is_ok());
        // if proof.is_err() {
        //     console_log!("{:?}", proof.map_err(JsValue::from).err().unwrap());
        // }
        let proof = proof.map_err(JsValue::from).unwrap();
        assert_eq!(proof.serialize()[0], 0b0001_0001);
        assert_eq!(proof.webauthn_fields_flags(), 0b0000_0000);
        assert_eq!(proof.webauthn_extra_fields().is_empty(), true);
    }

    /// Tests a signature generated with Android Chrome, which has no crossOrigin field in the client data JSON
    /// and has escaped forward slashes in the origin. It also has extra fields in the client data JSON.
    #[wasm_bindgen_test]
    fn it_can_create_a_nonstandard_webauthn_signature_proof() {
        let public_key = ES256PublicKey::new(&[
            3, 39, 225, 247, 153, 91, 222, 93, 248, 162, 43, 217, 194, 120, 51, 181, 50, 215, 156,
            35, 80, 230, 31, 201, 168, 86, 33, 209, 67, 142, 171, 235, 124,
        ])
        .map_err(JsValue::from)
        .unwrap();
        let signature = ES256Signature::from_asn1(&[
            48, 68, 2, 32, 113, 93, 20, 133, 221, 14, 184, 248, 122, 252, 45, 231, 122, 117, 249,
            13, 81, 25, 193, 158, 66, 9, 4, 25, 189, 42, 84, 211, 52, 255, 185, 16, 2, 32, 75, 245,
            24, 65, 236, 120, 114, 212, 34, 104, 105, 109, 80, 116, 252, 38, 53, 172, 76, 237, 183,
            46, 144, 179, 241, 138, 5, 61, 235, 172, 178, 252,
        ])
        .map_err(JsValue::from)
        .unwrap();
        let authenticator_data = &[
            122, 3, 161, 109, 254, 12, 67, 88, 183, 158, 235, 228, 242, 92, 186, 86, 236, 122, 167,
            200, 51, 31, 70, 169, 105, 136, 0, 109, 180, 64, 230, 144, 5, 0, 0, 0, 60,
        ];
        let client_data_json = &[
            123, 34, 116, 121, 112, 101, 34, 58, 34, 119, 101, 98, 97, 117, 116, 104, 110, 46, 103,
            101, 116, 34, 44, 34, 99, 104, 97, 108, 108, 101, 110, 103, 101, 34, 58, 34, 79, 67,
            65, 106, 97, 88, 69, 88, 115, 45, 80, 52, 122, 113, 69, 107, 45, 54, 49, 77, 104, 87,
            80, 79, 113, 57, 97, 56, 86, 112, 68, 76, 50, 113, 102, 51, 90, 119, 110, 65, 73, 57,
            73, 34, 44, 34, 111, 114, 105, 103, 105, 110, 34, 58, 34, 104, 116, 116, 112, 115, 58,
            92, 47, 92, 47, 119, 101, 98, 97, 117, 116, 104, 110, 46, 112, 111, 115, 46, 110, 105,
            109, 105, 113, 119, 97, 116, 99, 104, 46, 99, 111, 109, 34, 44, 34, 97, 110, 100, 114,
            111, 105, 100, 80, 97, 99, 107, 97, 103, 101, 78, 97, 109, 101, 34, 58, 34, 99, 111,
            109, 46, 97, 110, 100, 114, 111, 105, 100, 46, 99, 104, 114, 111, 109, 101, 34, 125,
        ];
        let proof = SignatureProof::webauthn_single_sig(
            &JsValue::from(public_key).into(),
            &JsValue::from(signature).into(),
            authenticator_data,
            client_data_json,
        );
        assert!(proof.is_ok());
        // if proof.is_err() {
        //     console_log!("{:?}", proof.map_err(JsValue::from).err().unwrap());
        // }
        let proof = proof.map_err(JsValue::from).unwrap();
        assert_eq!(proof.serialize()[0], 0b0001_0001);
        assert_eq!(proof.webauthn_fields_flags(), 0b0000_0011);
        assert_eq!(proof.webauthn_extra_fields().is_empty(), false);
    }
}
