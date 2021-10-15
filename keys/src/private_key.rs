use std::convert::{TryFrom, TryInto};
use std::fmt::{Debug, Error, Formatter};
use std::io;
use std::str::FromStr;

use hex::FromHex;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Hash, SerializeContent};
use rand_core::{CryptoRng, RngCore};
use utils::key_rng::SecureGenerate;

use crate::errors::{KeysError, ParseError};

pub struct PrivateKey(pub(super) ed25519_zebra::SigningKey);

impl PrivateKey {
    pub const SIZE: usize = 32;

    #[inline]
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0
            .as_ref()
            .try_into()
            .expect("Obtained slice with an unexpected size")
    }

    #[inline]
    pub(crate) fn as_zebra(&self) -> &ed25519_zebra::SigningKey {
        &self.0
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KeysError> {
        Ok(PrivateKey(ed25519_zebra::SigningKey::try_from(bytes)?))
    }

    #[inline]
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_bytes())
    }
}

impl SecureGenerate for PrivateKey {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        PrivateKey(ed25519_zebra::SigningKey::new(rng))
    }
}

impl<'a> From<&'a [u8; PrivateKey::SIZE]> for PrivateKey {
    fn from(bytes: &'a [u8; PrivateKey::SIZE]) -> Self {
        PrivateKey(ed25519_zebra::SigningKey::from(*bytes))
    }
}

impl From<[u8; PrivateKey::SIZE]> for PrivateKey {
    fn from(bytes: [u8; PrivateKey::SIZE]) -> Self {
        PrivateKey::from(&bytes)
    }
}

impl Clone for PrivateKey {
    fn clone(&self) -> Self {
        PrivateKey(self.0.clone())
    }
}

impl Deserialize for PrivateKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut buf = [0u8; PrivateKey::SIZE];
        reader.read_exact(&mut buf)?;
        Ok(PrivateKey::from(&buf))
    }
}

impl Serialize for PrivateKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write_all(self.as_bytes())?;
        Ok(self.serialized_size())
    }

    fn serialized_size(&self) -> usize {
        PrivateKey::SIZE
    }
}

impl SerializeContent for PrivateKey {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for PrivateKey {}

impl std::hash::Hash for PrivateKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.as_bytes(), state);
    }
}

impl Debug for PrivateKey {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        write!(f, "PrivateKey")
    }
}

impl PartialEq for PrivateKey {
    fn eq(&self, other: &PrivateKey) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for PrivateKey {}

impl FromHex for PrivateKey {
    type Error = ParseError;

    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<PrivateKey, ParseError> {
        Ok(PrivateKey::from_bytes(hex::decode(hex)?.as_slice())?)
    }
}

impl FromStr for PrivateKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PrivateKey::from_hex(s)
    }
}

impl Default for PrivateKey {
    fn default() -> Self {
        let default_array: [u8; Self::SIZE] = Default::default();
        Self::from(default_array)
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::borrow::Cow;

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::PrivateKey;

    impl Serialize for PrivateKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&self.to_hex())
        }
    }

    impl<'de> Deserialize<'de> for PrivateKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
            data.parse().map_err(Error::custom)
        }
    }
}
