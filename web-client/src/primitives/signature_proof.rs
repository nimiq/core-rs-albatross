use nimiq_hash::Hasher;
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
        authenticator_data: &[u8],
        client_data_json: &[u8],
    ) -> Result<SignatureProof, JsError> {
        // Extract host, client_data_flags and client_data_extra_fields from client_data_json

        // Setup inner fields
        let mut client_data_flags = nimiq_transaction::WebauthnClientDataFlags::default();
        let mut client_data_extra_fields = "".to_string();

        // Convert client_data_json bytes to string for search & split operations
        // FIXME: Handle invalid UTF-8
        let client_data_json_str = std::str::from_utf8(client_data_json).unwrap();

        // Extract origin from client_data_json
        let origin_search_term = r#","origin":""#;
        // Find the start of the origin value
        // FIXME: Handle missing origin
        let origin_start =
            client_data_json_str.find(origin_search_term).unwrap() + origin_search_term.len();
        // Find the closing quotation mark of the origin value
        // FIXME: Handle missing closing quotation mark
        let origin_length = client_data_json_str[origin_start..].find('"').unwrap();
        // The origin is the string between the two indices
        let origin = &client_data_json_str[origin_start..origin_start + origin_length];

        // Compute and compare RP ID with authenticatorData
        let url = url::Url::parse(origin)?;
        let hostname = url.host_str().unwrap(); // FIXME: Handle missing hostname
        let rp_id = nimiq_hash::Sha256Hasher::default().digest(hostname.as_bytes());
        if rp_id.0 != authenticator_data[0..32] {
            return Err(JsError::new(
                "Computed RP ID does not match authenticator data",
            ));
        }

        // Compute host field, which includes the port if non-standard
        let port_suffix = if let Some(port) = url.port() {
            format!(":{}", port)
        } else {
            "".to_string()
        };
        let host = format!("{}{}", hostname, port_suffix);

        // Check if client_data_json contains any extra fields
        // Search for the crossOrigin field first
        let parts = client_data_json_str
            .split(r#""crossOrigin":false"#)
            .collect::<Vec<_>>();
        let suffix = if parts.len() == 2 {
            parts[1]
        } else {
            // Client data does not include the crossOrigin field
            client_data_flags
                .insert(nimiq_transaction::WebauthnClientDataFlags::NO_CROSSORIGIN_FIELD);

            // We need to check for extra fields after the origin field instead
            let parts = client_data_json_str
                .split(&format!(r#""origin":"{}""#, origin))
                .collect::<Vec<_>>();
            assert_eq!(parts.len(), 2);
            parts[1]
        };

        // Check if the suffix contains extra fields
        if suffix.len() > 1 {
            // Cut off first comma and last curly brace
            client_data_extra_fields = suffix[1..suffix.len() - 1].to_string();
        }

        if origin.contains(r":\/\/") {
            client_data_flags
                .insert(nimiq_transaction::WebauthnClientDataFlags::ESCAPED_ORIGIN_SLASHES);
        }

        Ok(SignatureProof::from_native(
            nimiq_transaction::SignatureProof::ECDSA(
                nimiq_transaction::WebauthnSignatureProof::from(
                    *public_key.native_ref(),
                    signature.native_ref().clone(),
                    host,
                    authenticator_data[32..].to_vec(),
                    client_data_flags,
                    client_data_extra_fields,
                ),
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

#[cfg(test)]
mod tests {
    use wasm_bindgen::prelude::JsValue;
    use wasm_bindgen_test::*;

    use crate::primitives::{
        signature::Signature, signature_proof::SignatureProof,
        webauthn_public_key::WebauthnPublicKey,
    };

    /// Tests a signature generated with Desktop Chrome, which follows the Webauthn standard.
    #[wasm_bindgen_test]
    fn it_can_create_a_standard_webauthn_signature_proof() {
        let public_key = WebauthnPublicKey::new(&[
            2, 175, 112, 174, 46, 130, 50, 235, 92, 162, 248, 164, 196, 122, 113, 217, 205, 110,
            166, 47, 85, 36, 103, 240, 211, 197, 100, 40, 190, 71, 214, 56, 8,
        ])
        .map_err(JsValue::from)
        .unwrap();
        let signature = Signature::from_asn1(&[
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
            &public_key,
            &signature,
            authenticator_data,
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
        let public_key = WebauthnPublicKey::new(&[
            3, 39, 225, 247, 153, 91, 222, 93, 248, 162, 43, 217, 194, 120, 51, 181, 50, 215, 156,
            35, 80, 230, 31, 201, 168, 86, 33, 209, 67, 142, 171, 235, 124,
        ])
        .map_err(JsValue::from)
        .unwrap();
        let signature = Signature::from_asn1(&[
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
            &public_key,
            &signature,
            authenticator_data,
            client_data_json,
        );
        assert!(proof.is_ok());
        // if proof.is_err() {
        //     console_log!("{:?}", proof.map_err(JsValue::from).err().unwrap());
        // }
    }
}
