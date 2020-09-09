use std::cmp::Ordering;
use std::fmt;
use std::io;
use std::str::FromStr;

use ed25519_dalek::Verifier;
use hex::FromHex;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Hash, SerializeContent};

use crate::errors::{KeysError, ParseError};
use crate::{PrivateKey, Signature};

#[derive(Default, Eq, PartialEq, Clone, Copy)]
pub struct PublicKey(pub(super) ed25519_dalek::PublicKey);

impl PublicKey {
    pub const SIZE: usize = 32;

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        self.as_dalek().verify(data, signature.as_dalek()).is_ok()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; PublicKey::SIZE] {
        self.0.as_bytes()
    }

    #[inline]
    pub(crate) fn as_dalek(&self) -> &ed25519_dalek::PublicKey {
        &self.0
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KeysError> {
        Ok(PublicKey(ed25519_dalek::PublicKey::from_bytes(bytes)?))
    }

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

impl Ord for PublicKey {
    fn cmp(&self, other: &PublicKey) -> Ordering {
        self.0.as_bytes().cmp(other.0.as_bytes())
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> From<&'a PrivateKey> for PublicKey {
    fn from(private_key: &'a PrivateKey) -> Self {
        let public_key = ed25519_dalek::PublicKey::from(private_key.as_dalek());
        PublicKey(public_key)
    }
}

impl<'a> From<&'a [u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        PublicKey(ed25519_dalek::PublicKey::from_bytes(bytes).unwrap())
    }
}

impl From<[u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: [u8; PublicKey::SIZE]) -> Self {
        PublicKey::from(&bytes)
    }
}

impl Deserialize for PublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut buf = [0u8; PublicKey::SIZE];
        reader.read_exact(&mut buf)?;
        Ok(PublicKey::from_bytes(&buf).map_err(|_| SerializingError::InvalidValue)?)
    }
}

impl Serialize for PublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write_all(self.as_bytes())?;
        Ok(self.serialized_size())
    }

    fn serialized_size(&self) -> usize {
        PublicKey::SIZE
    }
}

impl SerializeContent for PublicKey {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

// This is a different Hash than the std Hash.
#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for PublicKey {}
