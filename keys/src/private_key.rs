use std::fmt::Debug;
use std::fmt::Error;
use std::fmt::Formatter;
use std::io;
use std::str::FromStr;

use ed25519_dalek;
use hex;
use hex::FromHex;
use rand::rngs::OsRng;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Hash, SerializeContent};

use crate::PublicKey;

#[derive(Default)]
pub struct PrivateKey(pub(in super) ed25519_dalek::SecretKey);

impl PrivateKey {
    pub const SIZE: usize = 32;

    pub fn generate() -> Self {
        PrivateKey(ed25519_dalek::SecretKey::generate(&mut OsRng))
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; PrivateKey::SIZE] { self.0.as_bytes() }

    #[inline]
    pub (crate) fn as_dalek(&self) -> &ed25519_dalek::SecretKey { &self.0 }

    #[inline]
    pub fn to_hex(&self) -> String { hex::encode(self.as_bytes()) }
}

impl<'a> From<&'a [u8; PrivateKey::SIZE]> for PrivateKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        PrivateKey(ed25519_dalek::SecretKey::from_bytes(bytes).unwrap())
    }
}

impl From<[u8; PrivateKey::SIZE]> for PrivateKey {
    fn from(bytes: [u8; PrivateKey::SIZE]) -> Self {
        PrivateKey::from(&bytes)
    }
}

impl Clone for PrivateKey {
    fn clone(&self) -> Self {
        let cloned_dalek = ed25519_dalek::SecretKey::from_bytes(self.0.as_bytes()).unwrap();
        PrivateKey(cloned_dalek)
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
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for PrivateKey { }

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

impl FromStr for PrivateKey {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let vec = Vec::from_hex(s)?;
        Ok(Deserialize::deserialize_from_vec(&vec)
            .map_err(|_| hex::FromHexError::InvalidStringLength)?)
    }
}
