#[cfg(feature = "beserial")]
use std::str::FromStr;
use std::{cmp::Ordering, fmt, io::Error};

use ark_ec::AffineCurve;
use ark_mnt6_753::G2Affine;

#[cfg(feature = "beserial")]
use beserial::Deserialize;

use crate::compression::BeDeserialize;
#[cfg(feature = "beserial")]
use crate::ParseError;
use crate::PublicKey;

/// The serialized compressed form of a public key.
/// This form consists of the x-coordinate of the point (in the affine form),
/// one bit indicating the sign of the y-coordinate
/// and one bit indicating if it is the "point-at-infinity".
#[derive(Clone)]
pub struct CompressedPublicKey {
    pub public_key: [u8; 285],
}

impl CompressedPublicKey {
    pub const SIZE: usize = 285;

    /// Transforms the compressed form back into the projective form.
    pub fn uncompress(&self) -> Result<PublicKey, Error> {
        let affine_point: G2Affine = BeDeserialize::deserialize(&mut &self.public_key[..])?;
        Ok(PublicKey {
            public_key: affine_point.into_projective(),
        })
    }

    /// Formats the compressed form into a hexadecimal string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_ref())
    }
}

impl Eq for CompressedPublicKey {}

impl PartialEq for CompressedPublicKey {
    fn eq(&self, other: &CompressedPublicKey) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Ord for CompressedPublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl PartialOrd<CompressedPublicKey> for CompressedPublicKey {
    fn partial_cmp(&self, other: &CompressedPublicKey) -> Option<Ordering> {
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl AsRef<[u8]> for CompressedPublicKey {
    fn as_ref(&self) -> &[u8] {
        self.public_key.as_ref()
    }
}

impl fmt::Display for CompressedPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", &self.to_hex())
    }
}

impl fmt::Debug for CompressedPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "CompressedPublicKey({})", &self.to_hex())
    }
}

#[cfg(feature = "beserial")]
impl FromStr for CompressedPublicKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = hex::decode(s)?;
        if raw.len() != CompressedPublicKey::SIZE {
            return Err(ParseError::IncorrectLength(raw.len()));
        }
        Ok(CompressedPublicKey::deserialize_from_vec(&raw).unwrap())
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    // TODO: Replace this with a generic serialization using `ToHex` and `FromHex`.

    use std::{borrow::Cow, str::FromStr};

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::CompressedPublicKey;

    impl Serialize for CompressedPublicKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&self.to_hex())
        }
    }

    impl<'de> Deserialize<'de> for CompressedPublicKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let s: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
            CompressedPublicKey::from_str(&s).map_err(Error::custom)
        }
    }
}
