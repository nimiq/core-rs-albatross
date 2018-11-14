use ed25519_dalek;
use beserial::{Serialize, SerializingError, Deserialize, ReadBytesExt, WriteBytesExt};

#[derive(Debug, Clone)]
pub struct Signature(pub(in super) ed25519_dalek::Signature);

impl Signature {
    pub const SIZE: usize = 64;

    #[inline]
    pub fn to_bytes(&self) -> [u8; Signature::SIZE] { self.0.to_bytes() }

    #[inline]
    pub(crate) fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::Signature { &self.0 }
}

impl Eq for Signature {}

impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        return self.0 == other.0;
    }
}

impl<'a> From<&'a [u8; Signature::SIZE]> for Signature {
    fn from(bytes: &'a [u8; Signature::SIZE]) -> Self {
        return Signature(ed25519_dalek::Signature::from_bytes(bytes).unwrap());
    }
}

impl From<[u8; Signature::SIZE]> for Signature {
    fn from(bytes: [u8; Signature::SIZE]) -> Self {
        return Signature::from(&bytes);
    }
}

impl Deserialize for Signature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut buf = [0u8; Signature::SIZE];
        reader.read_exact(&mut buf)?;
        return Ok(Signature::from(&buf));
    }
}

impl Serialize for Signature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write(&self.to_bytes())?;
        return Ok(self.serialized_size());
    }

    fn serialized_size(&self) -> usize {
        return Signature::SIZE;
    }
}
