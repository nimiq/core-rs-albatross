use std::cmp::Ordering;
use std::io;

use ed25519_dalek;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

use crate::{PrivateKey, Signature};
use hash::{Hash, SerializeContent};
use crate::errors::KeysError;

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub struct PublicKey(pub(in super) ed25519_dalek::PublicKey);

impl PublicKey {
    pub const SIZE: usize = 32;

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        return self.as_dalek().verify(data, signature.as_dalek()).is_ok();
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; PublicKey::SIZE] { self.0.as_bytes() }

    #[inline]
    pub(crate) fn as_dalek(&self) -> &ed25519_dalek::PublicKey { &self.0 }

    #[inline]
    pub fn from_bytes(bytes: &[u8; PublicKey::SIZE]) -> Result<Self, KeysError> {
        Ok(PublicKey(ed25519_dalek::PublicKey::from_bytes(bytes).map_err(|e| KeysError(e))?))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &PublicKey) -> Ordering {
        return self.0.as_bytes().cmp(other.0.as_bytes());
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        return Some(self.cmp(other));
    }
}

impl<'a> From<&'a PrivateKey> for PublicKey {
    fn from(private_key: &'a PrivateKey) -> Self {
        let public_key = ed25519_dalek::PublicKey::from(private_key.as_dalek());
        return PublicKey(public_key);
    }
}

impl<'a> From<&'a [u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        PublicKey(ed25519_dalek::PublicKey::from_bytes(bytes).unwrap())
    }
}

impl From<[u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: [u8; PublicKey::SIZE]) -> Self {
        return PublicKey::from(&bytes);
    }
}

impl Deserialize for PublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut buf = [0u8; PublicKey::SIZE];
        reader.read_exact(&mut buf)?;
        return Ok(PublicKey::from_bytes(&buf).map_err(|_| SerializingError::InvalidValue)?);
    }
}

impl Serialize for PublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write(self.as_bytes())?;
        return Ok(self.serialized_size());
    }

    fn serialized_size(&self) -> usize {
        return PublicKey::SIZE;
    }
}

impl SerializeContent for PublicKey {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for PublicKey { }

