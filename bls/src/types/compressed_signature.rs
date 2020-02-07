use super::*;

#[derive(Clone, Copy)]
pub struct CompressedSignature {
    pub(crate) signature: [u8; 48],
}

impl CompressedSignature {
    pub const SIZE: usize = 48;

    pub fn uncompress(&self) -> Result<Signature, SerializationError> {
        let affine_point = G1Affine::deserialize(&self.signature, &mut [])?;
        Ok(Signature {
            signature: affine_point.into_projective(),
        })
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.signature.as_ref())
    }
}

impl Eq for CompressedSignature {}

impl PartialEq for CompressedSignature {
    fn eq(&self, other: &CompressedSignature) -> bool {
        self.signature.as_ref() == other.signature.as_ref()
    }
}

impl Ord for CompressedSignature {
    fn cmp(&self, other: &Self) -> Ordering {
        self.signature.as_ref().cmp(other.signature.as_ref())
    }
}

impl PartialOrd<CompressedSignature> for CompressedSignature {
    fn partial_cmp(&self, other: &CompressedSignature) -> Option<Ordering> {
        self.signature
            .as_ref()
            .partial_cmp(other.signature.as_ref())
    }
}

impl Default for CompressedSignature {
    fn default() -> Self {
        CompressedSignature {
            signature: [0u8; 48],
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
