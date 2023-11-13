use std::{convert::TryFrom, fmt, str::FromStr};

use hex::FromHex;

use crate::errors::{ParseError, SignatureError};

#[derive(Debug, Clone)]
pub struct Ed25519Signature(pub(super) ed25519_zebra::Signature);

impl Ed25519Signature {
    pub const SIZE: usize = 64;

    #[inline]
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        self.0.into()
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SignatureError> {
        Ok(Ed25519Signature(ed25519_zebra::Signature::try_from(bytes)?))
    }

    #[inline]
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }
}

impl fmt::Display for Ed25519Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.to_hex())
    }
}

impl Eq for Ed25519Signature {}

impl Default for Ed25519Signature {
    fn default() -> Self {
        Ed25519Signature(ed25519_zebra::Signature::from([0u8; 64]))
    }
}

impl PartialEq for Ed25519Signature {
    fn eq(&self, other: &Self) -> bool {
        <[u8; 64]>::from(self.0) == <[u8; 64]>::from(other.0)
    }
}

impl FromHex for Ed25519Signature {
    type Error = ParseError;

    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Ed25519Signature, ParseError> {
        Ok(Ed25519Signature::from_bytes(hex::decode(hex)?.as_slice())?)
    }
}

impl<'a> From<&'a [u8; Self::SIZE]> for Ed25519Signature {
    fn from(bytes: &'a [u8; Self::SIZE]) -> Self {
        Ed25519Signature::from(*bytes)
    }
}

impl From<[u8; Self::SIZE]> for Ed25519Signature {
    fn from(bytes: [u8; Self::SIZE]) -> Self {
        Ed25519Signature(ed25519_zebra::Signature::from(bytes))
    }
}

impl FromStr for Ed25519Signature {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ed25519Signature::from_hex(s)
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::borrow::Cow;

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::Ed25519Signature;

    impl Serialize for Ed25519Signature {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if serializer.is_human_readable() {
                serializer.serialize_str(&self.to_hex())
            } else {
                serde_big_array::BigArray::serialize(&self.to_bytes(), serializer)
            }
        }
    }

    impl<'de> Deserialize<'de> for Ed25519Signature {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                data.parse().map_err(Error::custom)
            } else {
                let buf: [u8; Ed25519Signature::SIZE] =
                    serde_big_array::BigArray::deserialize(deserializer)?;
                Self::from_bytes(&buf).map_err(|_| D::Error::custom("Invalid signature"))
            }
        }
    }
}
