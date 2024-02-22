use std::{convert::TryFrom, fmt, str::FromStr};

use hex::FromHex;

use crate::errors::{ParseError, SignatureError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ES256Signature(pub(super) p256::ecdsa::Signature);

impl ES256Signature {
    pub const SIZE: usize = 64;

    #[inline]
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        self.0.to_bytes().into()
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SignatureError> {
        Ok(ES256Signature(p256::ecdsa::Signature::try_from(bytes)?))
    }

    #[inline]
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }
}

impl fmt::Display for ES256Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.to_hex())
    }
}

impl Default for ES256Signature {
    fn default() -> Self {
        ES256Signature(
            p256::ecdsa::Signature::from_bytes(&[0u8; Self::SIZE].into())
                .expect("Invalid slice length"),
        )
    }
}

impl FromHex for ES256Signature {
    type Error = ParseError;

    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<ES256Signature, ParseError> {
        Ok(ES256Signature::from_bytes(hex::decode(hex)?.as_slice())?)
    }
}

impl<'a> From<&'a [u8; Self::SIZE]> for ES256Signature {
    fn from(bytes: &'a [u8; Self::SIZE]) -> Self {
        ES256Signature::from(*bytes)
    }
}

impl From<[u8; Self::SIZE]> for ES256Signature {
    fn from(bytes: [u8; Self::SIZE]) -> Self {
        ES256Signature(
            p256::ecdsa::Signature::from_bytes(&bytes.into()).expect("Invalid slice length"),
        )
    }
}

impl FromStr for ES256Signature {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ES256Signature::from_hex(s)
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::borrow::Cow;

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::ES256Signature;

    impl Serialize for ES256Signature {
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

    impl<'de> Deserialize<'de> for ES256Signature {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                data.parse().map_err(Error::custom)
            } else {
                let buf: [u8; ES256Signature::SIZE] =
                    serde_big_array::BigArray::deserialize(deserializer)?;
                Self::from_bytes(&buf).map_err(|_| D::Error::custom("Invalid signature"))
            }
        }
    }
}
