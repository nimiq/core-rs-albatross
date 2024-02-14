use std::{cmp::Ord, error::Error, fmt};

use base64::Engine;
use bitflags::bitflags;
use nimiq_hash::{Blake2bHasher, Hasher, Sha256Hasher};
use nimiq_keys::{Address, Ed25519PublicKey, Ed25519Signature, PublicKey, Signature};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::merkle::Blake2bMerklePath;
use serde_json::json;
use url::Url;

#[derive(Clone, Debug)]
pub struct SignatureProof {
    pub public_key: PublicKey,
    pub merkle_path: Blake2bMerklePath,
    pub signature: Signature,
    pub webauthn_fields: Option<WebauthnExtraFields>,
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
        // Extract host, client_data_flags and client_data_extra_fields from client_data_json

        // Setup inner fields
        let mut client_data_flags = WebauthnClientDataFlags::default();
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
            return Err(SerializationError::new(
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
            client_data_flags.insert(WebauthnClientDataFlags::NO_CROSSORIGIN_FIELD);

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
            client_data_flags.insert(WebauthnClientDataFlags::ESCAPED_ORIGIN_SLASHES);
        }

        Ok(SignatureProof {
            public_key,
            merkle_path: Blake2bMerklePath::empty(),
            signature,
            webauthn_fields: Some(WebauthnExtraFields {
                host,
                authenticator_data_suffix: authenticator_data[32..].to_vec(),
                client_data_flags,
                client_data_extra_fields,
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
            self.do_verify(message)
        }
    }

    fn verify_webauthn(&self, message: &[u8]) -> bool {
        let webauthn_fields = self
            .webauthn_fields
            .as_ref()
            .expect("Webauthn fields not set");

        // Message in our case is a transaction's serialized content

        // 1. We need to hash the message to get our challenge data
        let challenge = Blake2bHasher::default().digest(message);

        // 2. We need to calculate the RP ID (Relaying Party ID) from the hostname (without port)
        // First we construct the origin from the host, as we need it for the clientDataJSON later
        let origin_protocol = if webauthn_fields.host.starts_with("localhost") {
            "http"
        } else {
            "https"
        };

        let protocol_separator = if webauthn_fields
            .client_data_flags
            .contains(WebauthnClientDataFlags::ESCAPED_ORIGIN_SLASHES)
        {
            r":\/\/"
        } else {
            "://"
        };

        let origin = format!(
            "{}{}{}",
            origin_protocol, protocol_separator, webauthn_fields.host
        );

        let url = Url::parse(&origin);
        if url.is_err() {
            debug!("Failed to parse origin: {}", origin);
            return false;
        }
        let url = url.unwrap();

        let hostname = url.host_str();
        if hostname.is_none() {
            debug!("Failed to extract hostname: {:?}", url);
            return false;
        }
        let hostname = hostname.unwrap();

        // The RP ID is the SHA256 hash of the hostname
        let rp_id = Sha256Hasher::default().digest(hostname.as_bytes());

        // 3. Build the authenticatorData from the RP ID and the suffix
        let authenticator_data =
            [rp_id.as_slice(), &webauthn_fields.authenticator_data_suffix].concat();

        // 4. Build the clientDataJSON from challenge and origin
        let mut client_data_json = json!({
            // Order of fields is defined by https://w3c.github.io/webauthn/#clientdatajson-serialization
            "type": "webauthn.get", // Do not support signing at credential registration, only at assertation
            "challenge": base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(challenge),
            "origin": origin,
            "crossOrigin": false, // Signing inside iframes is not supported
        })
        .to_string()
        // If the `origin` has escaped slashes, the json! macro re-escapes those escape-backslashes,
        // We remove the double escape here so the `origin` matches the expected value
        .replace(r"\\", r"\");

        if webauthn_fields
            .client_data_flags
            .contains(WebauthnClientDataFlags::NO_CROSSORIGIN_FIELD)
        {
            client_data_json = client_data_json.replace(r#","crossOrigin":false"#, "");
        }

        // Append extra clientData fields before the final closing bracket
        if !webauthn_fields.client_data_extra_fields.is_empty() {
            client_data_json = format!(
                "{},{}}}",
                &client_data_json[..client_data_json.len() - 1],
                webauthn_fields.client_data_extra_fields
            );
        }

        // Hash the clientDataJSON
        let client_data_hash = Sha256Hasher::default().digest(client_data_json.as_bytes());

        // 5. Concat authenticatorData and clientDataHash to build the data signed by Webauthn
        let signed_data = [authenticator_data.as_slice(), client_data_hash.as_slice()].concat();

        self.do_verify(signed_data.as_slice())
    }

    fn do_verify(&self, message: &[u8]) -> bool {
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

bitflags! {
    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    /// Some authenticators may behave non-standard for signing with Webauthn:
    ///
    /// - they might not include the mandatory `crossOrigin` field in clientDataJSON
    /// - they might escape the `origin`'s forward slashes with backslashes, although not necessary for UTF-8 nor JSON encoding
    ///
    /// To allow the WebauthnSignatureProof to construct a correct `clientDataJSON` for verification,
    /// the proof needs to know these non-standard behaviors.
    ///
    /// See this tracking issue for Android Chrome: https://bugs.chromium.org/p/chromium/issues/detail?id=1233616
    pub struct WebauthnClientDataFlags: u8 {
        const NO_CROSSORIGIN_FIELD  = 1 << 0;
        const ESCAPED_ORIGIN_SLASHES = 1 << 1;

        // const HAS_EXTRA_FIELDS = 1 << 7; // TODO Replace client_data_extra_fields length null byte when no extra fields are present
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebauthnExtraFields {
    pub host: String,
    pub authenticator_data_suffix: Vec<u8>,
    pub client_data_flags: WebauthnClientDataFlags,
    pub client_data_extra_fields: String,
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
