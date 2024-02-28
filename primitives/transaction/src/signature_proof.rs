use std::{cmp::Ord, error::Error, fmt, str};

use base64::prelude::{Engine as _, BASE64_URL_SAFE_NO_PAD};
use bitflags::bitflags;
use nimiq_hash::{Blake2bHash, Hash, HashOutput, Sha256Hash};
use nimiq_keys::{Address, Ed25519PublicKey, Ed25519Signature, PublicKey, Signature};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::merkle::Blake2bMerklePath;
use url::Url;

#[derive(Clone, Debug)]
pub struct SignatureProof {
    pub public_key: PublicKey,
    pub merkle_path: Blake2bMerklePath,
    pub signature: Signature,
    pub webauthn_fields: Option<WebauthnExtraFields>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebauthnClientData {
    #[serde(rename = "type")]
    type_: WebauthnClientDataType,
    challenge: String,
    origin: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    cross_origin: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum WebauthnClientDataType {
    #[serde(rename = "webauthn.get")]
    WebauthnGet,
}

impl SignatureProof {
    pub fn from(
        public_key: PublicKey,
        signature: Signature,
        webauthn_fields: Option<WebauthnExtraFields>,
    ) -> Self {
        SignatureProof {
            public_key,
            merkle_path: Blake2bMerklePath::empty(),
            signature,
            webauthn_fields,
        }
    }

    pub fn from_ed25519(public_key: Ed25519PublicKey, signature: Ed25519Signature) -> Self {
        SignatureProof {
            public_key: PublicKey::Ed25519(public_key),
            merkle_path: Blake2bMerklePath::empty(),
            signature: Signature::Ed25519(signature),
            webauthn_fields: None,
        }
    }

    pub fn try_from_webauthn(
        public_key: PublicKey,
        signature: Signature,
        authenticator_data: &[u8],
        client_data_json: &[u8],
    ) -> Result<Self, SerializationError> {
        let client_data_json_str = str::from_utf8(client_data_json)
            .map_err(|e| SerializationError::new(&format!("invalid UTF-8: {e}")))?;
        let client_data: WebauthnClientData = serde_json::from_str(client_data_json_str)
            .map_err(|e| SerializationError::new(&format!("invalid JSON: {e}")))?;

        let hostname = client_data
            .origin
            .host_str()
            .ok_or_else(|| SerializationError::new("missing hostname"))?;
        let rp_id: Sha256Hash = hostname.hash();

        if authenticator_data.len() < 32 {
            return Err(SerializationError::new("authenticator data too short"));
        }

        if *rp_id.as_bytes() != authenticator_data[0..32] {
            return Err(SerializationError::new(
                "Computed RP ID does not match authenticator data",
            ));
        }

        if client_data.cross_origin == Some(true) {
            return Err(SerializationError::new(
                "crossOrigin in clientDataJSON must be false",
            ));
        }

        Ok(SignatureProof {
            public_key,
            merkle_path: Blake2bMerklePath::empty(),
            signature,
            webauthn_fields: Some(WebauthnExtraFields {
                authenticator_data_suffix: authenticator_data[32..].to_vec(),
                client_data_json: String::from(client_data_json_str),
            }),
        })
    }

    pub fn compute_signer(&self) -> Address {
        let merkle_root = match self.public_key {
            PublicKey::Ed25519(ref public_key) => self.merkle_path.compute_root(public_key),
            PublicKey::ES256(ref public_key) => self.merkle_path.compute_root(public_key),
        };
        Address::from(merkle_root)
    }

    pub fn is_signed_by(&self, address: &Address) -> bool {
        self.compute_signer() == *address
    }

    pub fn verify(&self, message: &[u8]) -> bool {
        if self.webauthn_fields.is_some() {
            self.verify_webauthn(message)
        } else {
            self.verify_signature(message)
        }
    }

    fn verify_webauthn(&self, message: &[u8]) -> bool {
        let webauthn_fields = self
            .webauthn_fields
            .as_ref()
            .expect("Webauthn fields not set");

        let client_data: WebauthnClientData =
            match serde_json::from_str(&webauthn_fields.client_data_json) {
                Ok(client_data) => client_data,
                Err(e) => {
                    debug!("Failed to read client data JSON: {e}");
                    return false;
                }
            };

        let hostname = match client_data.origin.host_str() {
            Some(hostname) => hostname,
            None => {
                debug!("Missing hostname in client data JSON origin");
                return false;
            }
        };

        // Message in our case is a transaction's serialized content
        // 1. We need to hash the message to get our challenge data
        let challenge: Blake2bHash = message.hash();
        let challenge_base64 = BASE64_URL_SAFE_NO_PAD.encode(challenge);
        if challenge_base64 != client_data.challenge {
            debug!(
                "challenge mismatch: expected {}, got {}",
                client_data.challenge, challenge_base64,
            );
            return false;
        }

        // 2. The RP ID is the SHA256 hash of the hostname
        let rp_id: Sha256Hash = hostname.hash();

        // 3. Build the authenticatorData from the RP ID and the suffix
        let mut authenticator_data = Vec::new();
        authenticator_data.extend_from_slice(rp_id.as_slice());
        authenticator_data.extend_from_slice(&webauthn_fields.authenticator_data_suffix);

        // 4. Hash the clientDataJSON
        let client_data_hash: Sha256Hash = webauthn_fields.client_data_json.hash();

        // 5. Concat authenticatorData and clientDataHash to build the data signed by Webauthn
        let mut signed_data = authenticator_data;
        signed_data.extend_from_slice(client_data_hash.as_slice());

        self.verify_signature(&signed_data)
    }

    fn verify_signature(&self, message: &[u8]) -> bool {
        match self.public_key {
            PublicKey::Ed25519(ref public_key) => match self.signature {
                Signature::Ed25519(ref signature) => public_key.verify(signature, message),
                _ => false,
            },
            PublicKey::ES256(ref public_key) => match self.signature {
                Signature::ES256(ref signature) => public_key.verify(signature, message),
                _ => false,
            },
        }
    }

    pub fn make_type_and_flags_byte(&self) -> u8 {
        // Use the lower 4 bits for the algorithm variant
        let mut type_flags = match self.public_key {
            PublicKey::Ed25519(_) => SignatureProofAlgorithm::Ed25519,
            PublicKey::ES256(_) => SignatureProofAlgorithm::ES256,
        } as u8;

        // Use the upper 4 bits as flags
        let mut flags = SignatureProofFlags::default();
        if self.webauthn_fields.is_some() {
            flags.insert(SignatureProofFlags::WEBAUTHN_FIELDS);
        }
        type_flags |= flags.bits() << 4;

        type_flags
    }

    pub fn parse_type_and_flags_byte(
        byte: u8,
    ) -> Result<(SignatureProofAlgorithm, SignatureProofFlags), String> {
        // The algorithm is encoded in the lower 4 bits
        let type_byte = byte & 0b0000_1111;
        let algorithm = match type_byte {
            0 => SignatureProofAlgorithm::Ed25519,
            1 => SignatureProofAlgorithm::ES256,
            _ => return Err(format!("Invalid signature proof algorithm: {}", type_byte)),
        };
        // The flags are encoded in the upper 4 bits
        let flags = SignatureProofFlags::from_bits_truncate(byte >> 4);

        Ok((algorithm, flags))
    }
}

impl Default for SignatureProof {
    /// Default to Ed25519 public key and signature without Webauthn fields for backwards compatibility
    fn default() -> Self {
        SignatureProof {
            public_key: PublicKey::Ed25519(Default::default()),
            merkle_path: Default::default(),
            signature: Signature::Ed25519(Default::default()),
            webauthn_fields: None,
        }
    }
}

#[repr(u8)]
pub enum SignatureProofAlgorithm {
    Ed25519,
    ES256,
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    /// Store flags for serialized signature proofs. Can only use 4 bits,
    /// because the flags are stored in the upper 4 bits of the `type` field.
    pub struct SignatureProofFlags: u8 {
        const WEBAUTHN_FIELDS = 1 << 0;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebauthnExtraFields {
    pub authenticator_data_suffix: Vec<u8>,
    pub client_data_json: String,
}

mod serde_derive {
    use std::fmt;

    use nimiq_keys::{
        ES256PublicKey, ES256Signature, Ed25519PublicKey, Ed25519Signature, PublicKey, Signature,
    };
    use serde::ser::SerializeStruct;

    use super::*;

    const STRUCT_NAME: &str = "SignatureProof";

    const FIELDS: &[&str] = &[
        "type_and_flags",
        "public_key",
        "merkle_path",
        "signature",
        "webauthn_fields",
    ];

    impl serde::Serialize for SignatureProof {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut length = 4; // type field (algorithm & flags), public key, merkle path, signature
            if self.webauthn_fields.is_some() {
                length += 1; // Webauthn fields
            }

            let mut state = serializer.serialize_struct(STRUCT_NAME, length)?;

            state.serialize_field(FIELDS[0], &self.make_type_and_flags_byte())?;

            // Serialize public key without enum variant, as that is already encoded in the `type`/`algorithm` field
            match self.public_key {
                PublicKey::Ed25519(ref public_key) => {
                    state.serialize_field(FIELDS[1], public_key)?;
                }
                PublicKey::ES256(ref public_key) => {
                    state.serialize_field(FIELDS[1], public_key)?;
                }
            }

            // Serialize merkle path as is
            state.serialize_field(FIELDS[2], &self.merkle_path)?;

            // Serialize signature without enum variant, as that is already encoded in the `type`/`algorithm` field
            match self.signature {
                Signature::Ed25519(ref signature) => {
                    state.serialize_field(FIELDS[3], signature)?;
                }
                Signature::ES256(ref signature) => {
                    state.serialize_field(FIELDS[3], signature)?;
                }
            }

            // When present, serialize webauthn fields flattened into the root struct. The option variant is
            // encoded in the `type` field.
            if self.webauthn_fields.is_some() {
                state.serialize_field(FIELDS[4], self.webauthn_fields.as_ref().unwrap())?;
            }

            state.end()
        }
    }

    struct SignatureProofVisitor;

    impl<'de> serde::Deserialize<'de> for SignatureProof {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_struct(STRUCT_NAME, FIELDS, SignatureProofVisitor)
        }
    }

    impl<'de> serde::de::Visitor<'de> for SignatureProofVisitor {
        type Value = SignatureProof;

        fn expecting(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f, "a SignatureProof")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<SignatureProof, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let type_field: u8 = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;

            let (algorithm, flags) = SignatureProof::parse_type_and_flags_byte(type_field)
                .map_err(serde::de::Error::custom)?;

            let public_key = match algorithm {
                SignatureProofAlgorithm::Ed25519 => {
                    let public_key: Ed25519PublicKey = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                    PublicKey::Ed25519(public_key)
                }
                SignatureProofAlgorithm::ES256 => {
                    let public_key: ES256PublicKey = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                    PublicKey::ES256(public_key)
                }
            };

            let merkle_path: Blake2bMerklePath = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;

            let signature = match algorithm {
                SignatureProofAlgorithm::Ed25519 => {
                    let signature: Ed25519Signature = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(3, &self))?;
                    Signature::Ed25519(signature)
                }
                SignatureProofAlgorithm::ES256 => {
                    let signature: ES256Signature = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(3, &self))?;
                    Signature::ES256(signature)
                }
            };

            let webauthn_fields = if flags.contains(SignatureProofFlags::WEBAUTHN_FIELDS) {
                Some(
                    seq.next_element::<WebauthnExtraFields>()?
                        .ok_or_else(|| serde::de::Error::invalid_length(4, &self))?,
                )
            } else {
                None
            };

            Ok(SignatureProof {
                public_key,
                merkle_path,
                signature,
                webauthn_fields,
            })
        }
    }
}

#[derive(Debug)]
pub struct SerializationError {
    msg: String,
}

impl SerializationError {
    fn new(msg: &str) -> SerializationError {
        SerializationError {
            msg: msg.to_string(),
        }
    }
}

impl fmt::Display for SerializationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl Error for SerializationError {
    fn description(&self) -> &str {
        &self.msg
    }
}

impl From<url::ParseError> for SerializationError {
    fn from(err: url::ParseError) -> Self {
        SerializationError::new(&format!("Failed to parse URL: {}", err))
    }
}
