//! This modules implements `Serialize` and `Deserialize` for some types from [`libp2p`](https://docs.rs/libp2p).

use std::convert::TryFrom;

use byteorder::{ReadBytesExt, WriteBytesExt};
use libp2p::{
    identity::{Keypair, PublicKey},
    Multiaddr, PeerId,
};

use crate::{
    uvar, Deserialize, DeserializeWithLength, Serialize, SerializeWithLength, SerializingError,
};

impl Serialize for Multiaddr {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let raw: &[u8] = self.as_ref();
        SerializeWithLength::serialize::<uvar, W>(raw, writer)
    }

    fn serialized_size(&self) -> usize {
        let raw: &[u8] = self.as_ref();
        SerializeWithLength::serialized_size::<uvar>(raw)
    }
}

impl Deserialize for Multiaddr {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let buf: Vec<u8> = DeserializeWithLength::deserialize::<u8, R>(reader)?;
        Multiaddr::try_from(buf).map_err(|_| SerializingError::InvalidValue)
    }
}

impl Serialize for PublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            PublicKey::Ed25519(pk) => {
                writer.write_all(&pk.encode())?;
                Ok(32) // `PublicKey::encode` always returns a `[u8; 32]`.
            }
        }
    }

    fn serialized_size(&self) -> usize {
        32
    }
}

impl Deserialize for PublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf)?;
        let pk = libp2p::identity::ed25519::PublicKey::decode(&buf)
            .map_err(|_| SerializingError::InvalidValue)?;
        Ok(PublicKey::Ed25519(pk))
    }
}

impl Serialize for PeerId {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        SerializeWithLength::serialize::<u8, W>(&self.to_bytes(), writer)
    }

    fn serialized_size(&self) -> usize {
        SerializeWithLength::serialized_size::<u8>(&self.to_bytes())
    }
}

impl Deserialize for PeerId {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let raw: Vec<u8> = DeserializeWithLength::deserialize::<u8, R>(reader)?;
        PeerId::from_bytes(&raw).map_err(|_| SerializingError::InvalidValue)
    }
}

impl Serialize for Keypair {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            Keypair::Ed25519(keypair) => {
                writer.write_all(&keypair.encode())?;
                Ok(64)
            }
        }
    }

    fn serialized_size(&self) -> usize {
        64
    }
}

impl Deserialize for Keypair {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut buf = [0u8; 64];
        reader.read_exact(&mut buf)?;
        let pk = libp2p::identity::ed25519::Keypair::decode(&mut buf)
            .map_err(|_| SerializingError::InvalidValue)?;
        Ok(Keypair::Ed25519(pk))
    }
}
