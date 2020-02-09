use super::*;

/// The serialized compressed form of a public key. This form consists of the x-coordinate of the point (in the affine form), one bit indicating the sign of the y-coordinate, one bit indicating if it is the "point-at-infinity" and one bit indicating that this is the compressed form.
#[derive(Clone)]
pub struct CompressedPublicKey {
    pub public_key: [u8; 96],
}

impl CompressedPublicKey {
    pub const SIZE: usize = 96;

    /// Transforms the compressed form back into the projective form.
    pub fn uncompress(&self) -> Result<PublicKey, SerializationError> {
        let affine_point = G2Affine::deserialize(&self.public_key, &mut [])?;
        Ok(PublicKey {
            public_key: affine_point.into_projective(),
        })
    }

    /// Formats the compressed form into a hexadecimal string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_ref())
    }
}

impl Eq for CompressedPublicKey {}

impl PartialEq for CompressedPublicKey {
    fn eq(&self, other: &CompressedPublicKey) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Ord for CompressedPublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl PartialOrd<CompressedPublicKey> for CompressedPublicKey {
    fn partial_cmp(&self, other: &CompressedPublicKey) -> Option<Ordering> {
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl AsRef<[u8]> for CompressedPublicKey {
    fn as_ref(&self) -> &[u8] {
        self.public_key.as_ref()
    }
}

impl fmt::Display for CompressedPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", &self.to_hex())
    }
}

impl fmt::Debug for CompressedPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "CompressedPublicKey({})", &self.to_hex())
    }
}

#[cfg(feature = "beserial")]
impl FromStr for CompressedPublicKey {
    type Err = PublicKeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = hex::decode(s)?;
        if raw.len() != CompressedPublicKey::SIZE {
            return Err(PublicKeyParseError::IncorrectLength(raw.len()));
        }
        Ok(CompressedPublicKey::deserialize_from_vec(&raw).unwrap())
    }
}
