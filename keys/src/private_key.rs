use std::{
    convert::{TryFrom, TryInto},
    fmt::{Debug, Error, Formatter},
    str::FromStr,
};

use curve25519_dalek::scalar::Scalar;
use hex::FromHex;
use nimiq_utils::key_rng::SecureGenerate;
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest as _, Sha512};

use crate::errors::{KeysError, ParseError};

pub struct PrivateKey(pub ed25519_zebra::SigningKey);

impl PrivateKey {
    pub const SIZE: usize = 32;

    #[inline]
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0
            .as_ref()
            .try_into()
            .expect("Obtained slice with an unexpected size")
    }

    pub fn to_scalar(&self) -> Scalar {
        // Convert to scalar as in RFC 8032, section 6, `def secret_expand(secret)`:
        // https://www.rfc-editor.org/rfc/rfc8032.html#section-6
        let mut scalar_bytes = [0u8; 32];
        scalar_bytes.copy_from_slice(&Sha512::digest(self.as_bytes()).as_slice()[..32]);
        scalar_bytes[0] &= 248;
        scalar_bytes[31] &= 127;
        scalar_bytes[31] |= 64;
        Scalar::from_bits(scalar_bytes)
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
        PrivateKey(self.0)
    }
}

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
    use std::{borrow::Cow, io};

    use nimiq_hash::SerializeContent;
    use nimiq_serde::Serialize as NimiqSerialize;
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
            if serializer.is_human_readable() {
                serializer.serialize_str(&self.to_hex())
            } else {
                Serialize::serialize(&self.as_bytes(), serializer)
            }
        }
    }

    impl<'de> Deserialize<'de> for PrivateKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                data.parse().map_err(Error::custom)
            } else {
                let buf: [u8; PrivateKey::SIZE] = Deserialize::deserialize(deserializer)?;
                Ok(PrivateKey::from(&buf))
            }
        }
    }

    impl SerializeContent for PrivateKey {
        fn serialize_content<W: io::Write, H>(&self, writer: &mut W) -> io::Result<usize> {
            self.serialize_to_writer(writer)
        }
    }
}
