use std::{
    cmp::Ordering,
    convert::{TryFrom, TryInto},
    fmt,
    str::FromStr,
};

use hex::FromHex;

use crate::{
    errors::{KeysError, ParseError},
    PrivateKey, Signature,
};

#[derive(Clone, Copy)]
#[cfg_attr(feature = "serde-derive", derive(nimiq_hash_derive::SerializeContent))]
pub struct PublicKey(pub ed25519_zebra::VerificationKeyBytes);

impl PublicKey {
    pub const SIZE: usize = 32;

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        if let Ok(vk) = ed25519_zebra::VerificationKey::try_from(self.0) {
            vk.verify(&signature.0, data).is_ok()
        } else {
            false
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; PublicKey::SIZE] {
        self.0
            .as_ref()
            .try_into()
            .expect("Obtained slice with an unexpected size")
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KeysError> {
        Ok(PublicKey(ed25519_zebra::VerificationKeyBytes::try_from(
            bytes,
        )?))
    }

    #[inline]
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_bytes())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(self, f)
    }
}

impl FromHex for PublicKey {
    type Error = ParseError;

    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<PublicKey, ParseError> {
        Ok(PublicKey::from_bytes(hex::decode(hex)?.as_slice())?)
    }
}

impl FromStr for PublicKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PublicKey::from_hex(s)
    }
}

impl Default for PublicKey {
    fn default() -> Self {
        let default_array: [u8; Self::SIZE] = Default::default();
        Self::from(default_array)
    }
}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for PublicKey {}

impl Ord for PublicKey {
    fn cmp(&self, other: &PublicKey) -> Ordering {
        self.0.as_ref().cmp(other.0.as_ref())
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> From<&'a PrivateKey> for PublicKey {
    fn from(private_key: &'a PrivateKey) -> Self {
        let public_key = ed25519_zebra::VerificationKeyBytes::from(&private_key.0);
        PublicKey(public_key)
    }
}

impl<'a> From<&'a [u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        let public_key = ed25519_zebra::VerificationKeyBytes::from(*bytes);
        PublicKey(public_key)
    }
}

impl From<[u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: [u8; PublicKey::SIZE]) -> Self {
        PublicKey::from(&bytes)
    }
}

impl std::hash::Hash for PublicKey {
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

    use super::PublicKey;

    impl Serialize for PublicKey {
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

    impl<'de> Deserialize<'de> for PublicKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                data.parse().map_err(Error::custom)
            } else {
                let buf: [u8; PublicKey::SIZE] = Deserialize::deserialize(deserializer)?;
                PublicKey::from_bytes(&buf).map_err(|_| D::Error::custom("Invalid public key"))
            }
        }
    }
}
