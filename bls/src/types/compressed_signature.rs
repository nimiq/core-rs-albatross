use std::io::Error;
#[cfg(feature = "beserial")]
use std::str::FromStr;
use std::{cmp::Ordering, fmt};

use ark_ec::AffineCurve;
use ark_mnt6_753::G1Affine;

#[cfg(feature = "beserial")]
use beserial::Deserialize;

use crate::compression::BeDeserialize;
#[cfg(feature = "beserial")]
use crate::ParseError;
use crate::Signature;

/// The serialized compressed form of a signature.
/// This form consists of the x-coordinate of the point (in the affine form),
/// one bit indicating the sign of the y-coordinate
/// and one bit indicating if it is the "point-at-infinity".
#[derive(Clone, Copy)]
pub struct CompressedSignature {
    pub signature: [u8; 95],
}

impl CompressedSignature {
    pub const SIZE: usize = 95;

    /// Transforms the compressed form back into the projective form.
    pub fn uncompress(&self) -> Result<Signature, Error> {
        let affine_point: G1Affine = BeDeserialize::deserialize(&mut &self.signature[..])?;
        Ok(Signature {
            signature: affine_point.into_projective(),
        })
    }

    /// Formats the compressed form into a hexadecimal string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_ref())
    }
}

impl Eq for CompressedSignature {}

impl PartialEq for CompressedSignature {
    fn eq(&self, other: &CompressedSignature) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Ord for CompressedSignature {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl PartialOrd<CompressedSignature> for CompressedSignature {
    fn partial_cmp(&self, other: &CompressedSignature) -> Option<Ordering> {
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl Default for CompressedSignature {
    fn default() -> Self {
        CompressedSignature {
            signature: [0u8; CompressedSignature::SIZE],
        }
    }
}

impl fmt::Debug for CompressedSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "CompressedSignature({})", self.to_hex())
    }
}

impl fmt::Display for CompressedSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.to_hex())
    }
}

#[cfg(feature = "beserial")]
impl FromStr for CompressedSignature {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = hex::decode(s)?;
        if raw.len() != CompressedSignature::SIZE {
            return Err(ParseError::IncorrectLength(raw.len()));
        }
        Ok(CompressedSignature::deserialize_from_vec(&raw).unwrap())
    }
}

impl AsRef<[u8]> for CompressedSignature {
    fn as_ref(&self) -> &[u8] {
        self.signature.as_ref()
    }
}

impl AsMut<[u8]> for CompressedSignature {
    fn as_mut(&mut self) -> &mut [u8] {
        self.signature.as_mut()
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

    use super::CompressedSignature;

    impl Serialize for CompressedSignature {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&self.to_hex())
        }
    }

    impl<'de> Deserialize<'de> for CompressedSignature {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let s: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
            CompressedSignature::from_str(&s).map_err(Error::custom)
        }
    }
}
