use super::*;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Hash, SerializeContent};
use std::io;

use super::{
    AggregatePublicKey as GenericAggregatePublicKey,
    AggregateSignature as GenericAggregateSignature,
};

impl Serialize for CompressedPublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        assert_eq!(self.p_pub.as_ref().len(), CompressedPublicKey::SIZE);
        writer.write_all(self.p_pub.as_ref())?;
        Ok(CompressedPublicKey::SIZE)
    }

    fn serialized_size(&self) -> usize {
        CompressedPublicKey::SIZE
    }
}

impl SerializeContent for CompressedPublicKey {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for CompressedPublicKey {}

impl Deserialize for CompressedPublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0u8; CompressedPublicKey::SIZE];
        reader.read_exact(&mut bytes)?;

        let mut point = G2Compressed::empty();
        point.as_mut().copy_from_slice(&bytes);
        Ok(CompressedPublicKey { p_pub: point })
    }
}

impl Serialize for CompressedSignature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        assert_eq!(self.s.as_ref().len(), CompressedSignature::SIZE);
        writer.write_all(self.s.as_ref())?;
        Ok(CompressedSignature::SIZE)
    }

    fn serialized_size(&self) -> usize {
        CompressedSignature::SIZE
    }
}

impl SerializeContent for CompressedSignature {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for CompressedSignature {}

impl Deserialize for CompressedSignature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut bytes = [0u8; CompressedSignature::SIZE];
        reader.read_exact(&mut bytes)?;

        let mut point = G1Compressed::empty();
        point.as_mut().copy_from_slice(&bytes);
        Ok(CompressedSignature { s: point })
    }
}

impl Serialize for PublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        self.compress().serialize(writer)
    }

    fn serialized_size(&self) -> usize {
        CompressedPublicKey::SIZE
    }
}

impl SerializeContent for PublicKey {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for PublicKey {}

impl Deserialize for PublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let public_key: CompressedPublicKey = Deserialize::deserialize(reader)?;
        public_key
            .uncompress()
            .map_err(|_| SerializingError::InvalidValue)
    }
}

impl Serialize for Signature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        self.compress().serialize(writer)
    }

    fn serialized_size(&self) -> usize {
        CompressedSignature::SIZE
    }
}

impl SerializeContent for Signature {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for Signature {}

impl Deserialize for Signature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let signature: CompressedSignature = Deserialize::deserialize(reader)?;
        signature
            .uncompress()
            .map_err(|_| SerializingError::InvalidValue)
    }
}

impl Serialize for PublicKeyAffine {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        self.as_projective().serialize(writer)
    }

    fn serialized_size(&self) -> usize {
        CompressedPublicKey::SIZE
    }
}

impl Deserialize for PublicKeyAffine {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let public_key: PublicKey = Deserialize::deserialize(reader)?;
        Ok(public_key.as_affine())
    }
}

impl Serialize for SecretKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let repr = self.x.into_repr();
        repr.write_be(writer)?;
        Ok(SecretKey::SIZE)
    }

    fn serialized_size(&self) -> usize {
        SecretKey::SIZE
    }
}

impl Deserialize for SecretKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut element = FrRepr::default();
        element.read_be(reader)?;

        Ok(SecretKey {
            x: Fr::from_repr(element).map_err(|_| SerializingError::InvalidValue)?,
        })
    }
}

impl Serialize for AggregatePublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        self.0.serialize(writer)
    }

    fn serialized_size(&self) -> usize {
        self.0.serialized_size()
    }
}

impl Deserialize for AggregatePublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(GenericAggregatePublicKey(Deserialize::deserialize(reader)?))
    }
}

impl Serialize for AggregateSignature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        self.0.serialize(writer)
    }

    fn serialized_size(&self) -> usize {
        self.0.serialized_size()
    }
}

impl Deserialize for AggregateSignature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(GenericAggregateSignature(Deserialize::deserialize(reader)?))
    }
}

impl Serialize for KeyPair {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        self.secret.serialize(writer)
    }

    fn serialized_size(&self) -> usize {
        self.secret.serialized_size()
    }
}

impl Deserialize for KeyPair {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let secret: SecretKey = Deserialize::deserialize(reader)?;
        Ok(KeyPair::from(secret))
    }
}
