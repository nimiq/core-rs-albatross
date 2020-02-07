use super::*;

// The serialized compressed form of a signature. This form consists of the x-coordinate of the point (in the affine form), one bit indicating the sign of the y-coordinate, one bit indicating if it is the "point-at-infinity" and one bit indicating that this is the compressed form.
#[derive(Clone, Copy)]
pub struct CompressedSignature {
    pub(crate) signature: [u8; 48],
}

impl CompressedSignature {
    pub const SIZE: usize = 48;

    // Transforms the compressed form back into the projective form.
    pub fn uncompress(&self) -> Result<Signature, SerializationError> {
        let affine_point = G1Affine::deserialize(&self.signature, &mut [])?;
        Ok(Signature {
            signature: affine_point.into_projective(),
        })
    }

    // Formats the compressed form into a hexadecimal string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_ref())
    }
}

impl Eq for CompressedSignature {}

impl PartialEq for CompressedSignature {
    fn eq(&self, other: &CompressedSignature) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Ord for CompressedSignature {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl PartialOrd<CompressedSignature> for CompressedSignature {
    fn partial_cmp(&self, other: &CompressedSignature) -> Option<Ordering> {
        self.as_ref().partial_cmp(other.as_ref())
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
