use std::{
    convert::{TryFrom, TryInto},
    fmt,
    str::FromStr,
};

use hex::FromHex;

use crate::{
    errors::{KeysError, ParseError},
    Ed25519Signature, PrivateKey,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde-derive", derive(nimiq_hash_derive::SerializeContent))]
pub struct Ed25519PublicKey(pub ed25519_zebra::VerificationKeyBytes);

impl Ed25519PublicKey {
    pub const SIZE: usize = 32;

    pub fn verify(&self, signature: &Ed25519Signature, data: &[u8]) -> bool {
        if let Ok(vk) = ed25519_zebra::VerificationKey::try_from(self.0) {
            vk.verify(&signature.0, data).is_ok()
        } else {
            false
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; Ed25519PublicKey::SIZE] {
        self.0
            .as_ref()
            .try_into()
            .expect("Obtained slice with an unexpected size")
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KeysError> {
        Ok(Ed25519PublicKey(
            ed25519_zebra::VerificationKeyBytes::try_from(bytes)?,
        ))
    }

    #[inline]
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_bytes())
    }
}

impl fmt::Display for Ed25519PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Ed25519PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(self, f)
    }
}

impl FromHex for Ed25519PublicKey {
    type Error = ParseError;

    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Ed25519PublicKey, ParseError> {
        Ok(Ed25519PublicKey::from_bytes(hex::decode(hex)?.as_slice())?)
    }
}

impl FromStr for Ed25519PublicKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ed25519PublicKey::from_hex(s)
    }
}

impl Default for Ed25519PublicKey {
    fn default() -> Self {
        let default_array: [u8; Self::SIZE] = Default::default();
        Self::from(default_array)
    }
}

impl<'a> From<&'a PrivateKey> for Ed25519PublicKey {
    fn from(private_key: &'a PrivateKey) -> Self {
        let public_key = ed25519_zebra::VerificationKeyBytes::from(&private_key.0);
        Ed25519PublicKey(public_key)
    }
}

impl<'a> From<&'a [u8; Ed25519PublicKey::SIZE]> for Ed25519PublicKey {
    fn from(bytes: &'a [u8; Ed25519PublicKey::SIZE]) -> Self {
        let public_key = ed25519_zebra::VerificationKeyBytes::from(*bytes);
        Ed25519PublicKey(public_key)
    }
}

impl From<[u8; Ed25519PublicKey::SIZE]> for Ed25519PublicKey {
    fn from(bytes: [u8; Ed25519PublicKey::SIZE]) -> Self {
        Ed25519PublicKey::from(&bytes)
    }
}

impl std::hash::Hash for Ed25519PublicKey {
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

    use super::Ed25519PublicKey;

    impl Serialize for Ed25519PublicKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if serializer.is_human_readable() {
                serializer.serialize_str(&self.to_hex())
            } else {
                Serialize::serialize(self.as_bytes(), serializer)
            }
        }
    }

    impl<'de> Deserialize<'de> for Ed25519PublicKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                data.parse().map_err(Error::custom)
            } else {
                let buf: [u8; Ed25519PublicKey::SIZE] = Deserialize::deserialize(deserializer)?;
                Ed25519PublicKey::from_bytes(&buf)
                    .map_err(|_| D::Error::custom("Invalid public key"))
            }
        }
    }
}
