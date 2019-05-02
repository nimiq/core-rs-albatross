use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

use crate::bls12_381::*;
use crate::Encoding;

impl Serialize for PublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let bytes = self.to_bytes();
        writer.write(&bytes)?;
        Ok(PublicKey::SIZE)
    }

    fn serialized_size(&self) -> usize {
        PublicKey::SIZE
    }
}

impl Deserialize for PublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0u8; PublicKey::SIZE];
        reader.read_exact(&mut bytes)?;
        Ok(PublicKey::from_slice(&bytes).map_err(|_| SerializingError::InvalidValue)?)
    }
}

impl Serialize for SecretKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let bytes = self.to_bytes();
        writer.write(&bytes)?;
        Ok(SecretKey::SIZE)
    }

    fn serialized_size(&self) -> usize {
        SecretKey::SIZE
    }
}

impl Deserialize for SecretKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0u8; SecretKey::SIZE];
        reader.read_exact(&mut bytes)?;
        Ok(SecretKey::from_slice(&bytes).map_err(|_| SerializingError::InvalidValue)?)
    }
}

impl Serialize for Signature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let bytes = self.to_bytes();
        writer.write(&bytes)?;
        Ok(Signature::SIZE)
    }

    fn serialized_size(&self) -> usize {
        Signature::SIZE
    }
}

impl Deserialize for Signature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0u8; Signature::SIZE];
        reader.read_exact(&mut bytes)?;
        Ok(Signature::from_slice(&bytes).map_err(|_| SerializingError::InvalidValue)?)
    }
}

impl Serialize for AggregatePublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let bytes = self.to_bytes();
        writer.write(&bytes)?;
        Ok(AggregatePublicKey::SIZE)
    }

    fn serialized_size(&self) -> usize {
        AggregatePublicKey::SIZE
    }
}

impl Deserialize for AggregatePublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0u8; AggregatePublicKey::SIZE];
        reader.read_exact(&mut bytes)?;
        Ok(AggregatePublicKey::from_slice(&bytes).map_err(|_| SerializingError::InvalidValue)?)
    }
}

impl Serialize for AggregateSignature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let bytes = self.to_bytes();
        writer.write(&bytes)?;
        Ok(AggregateSignature::SIZE)
    }

    fn serialized_size(&self) -> usize {
        AggregateSignature::SIZE
    }
}

impl Deserialize for AggregateSignature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0u8; AggregateSignature::SIZE];
        reader.read_exact(&mut bytes)?;
        Ok(AggregateSignature::from_slice(&bytes).map_err(|_| SerializingError::InvalidValue)?)
    }
}
