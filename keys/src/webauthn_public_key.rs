use std::{cmp::Ordering, convert::TryInto, fmt, str::FromStr};

use hex::FromHex;

use crate::{
    errors::{KeysError, ParseError},
    Signature,
};

#[derive(Clone, Copy)]
#[cfg_attr(feature = "serde-derive", derive(nimiq_hash_derive::SerializeContent))]
pub struct WebauthnPublicKey(pub p256::EncodedPoint);

impl WebauthnPublicKey {
    pub const SIZE: usize = 33;

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        let signature = p256::ecdsa::Signature::from_slice(&signature.to_bytes()).unwrap();
        if let Ok(vk) = p256::ecdsa::VerifyingKey::from_encoded_point(&self.0) {
            p256::ecdsa::signature::Verifier::verify(&vk, data, &signature).is_ok()
        } else {
            false
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; WebauthnPublicKey::SIZE] {
        self.0
            .as_bytes()
            .try_into()
            .expect("Obtained slice with an unexpected size")
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KeysError> {
        p256::EncodedPoint::from_bytes(bytes)
            .map_err(|_| KeysError::MalformedPublicKey)
            .map(WebauthnPublicKey)
    }

    #[inline]
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_bytes())
    }
}

impl fmt::Display for WebauthnPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for WebauthnPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(self, f)
    }
}

impl FromHex for WebauthnPublicKey {
    type Error = ParseError;

    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<WebauthnPublicKey, ParseError> {
        Ok(WebauthnPublicKey::from_bytes(hex::decode(hex)?.as_slice())?)
    }
}

impl FromStr for WebauthnPublicKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        WebauthnPublicKey::from_hex(s)
    }
}

impl Default for WebauthnPublicKey {
    fn default() -> Self {
        let default_array = [0; Self::SIZE];
        Self::from(default_array)
    }
}

impl PartialEq for WebauthnPublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for WebauthnPublicKey {}

impl Ord for WebauthnPublicKey {
    fn cmp(&self, other: &WebauthnPublicKey) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for WebauthnPublicKey {
    fn partial_cmp(&self, other: &WebauthnPublicKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> From<&'a [u8; WebauthnPublicKey::SIZE]> for WebauthnPublicKey {
    fn from(bytes: &'a [u8; WebauthnPublicKey::SIZE]) -> Self {
        Self::from_bytes(bytes).expect("Unexpected size for")
    }
}

impl From<[u8; WebauthnPublicKey::SIZE]> for WebauthnPublicKey {
    fn from(bytes: [u8; WebauthnPublicKey::SIZE]) -> Self {
        WebauthnPublicKey::from(&bytes)
    }
}

impl std::hash::Hash for WebauthnPublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.as_bytes(), state);
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::borrow::Cow;

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::WebauthnPublicKey;

    impl Serialize for WebauthnPublicKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if serializer.is_human_readable() {
                serializer.serialize_str(&self.to_hex())
            } else {
                serde_big_array::BigArray::serialize(self.as_bytes(), serializer)
            }
        }
    }

    impl<'de> Deserialize<'de> for WebauthnPublicKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                data.parse().map_err(Error::custom)
            } else {
                let buf: [u8; WebauthnPublicKey::SIZE] =
                    serde_big_array::BigArray::deserialize(deserializer)?;
                WebauthnPublicKey::from_bytes(&buf)
                    .map_err(|_| D::Error::custom("Invalid public key"))
            }
        }
    }
}
