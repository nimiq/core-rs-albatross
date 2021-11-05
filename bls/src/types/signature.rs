use std::fmt;

use ark_ec::ProjectiveCurve;
use ark_mnt6_753::G1Projective;

use nimiq_hash::HashOutput;

use crate::pedersen::{pedersen_generators, pedersen_hash};
use crate::utils::bytes_to_bits;
use crate::{CompressedSignature, SigHash};

#[derive(Clone, Copy)]
pub struct Signature {
    /// The projective form is the longer one, with three coordinates. The
    /// affine form is the shorter one, with only two coordinates. Calculation
    /// is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support
    /// arithmetic with it.
    pub signature: G1Projective,
    /// Cache for the compressed form of the signature. This is done in order
    /// to optimize the serialization since it needs to be compressed for this
    /// purpose.
    pub compressed: CompressedSignature,
}

impl Signature {
    /// Maps an hash to a elliptic curve point in the G1 group, it is known as
    /// "hash-to-curve". It is required to create signatures. We use the
    /// Pedersen hash to create the EC point.
    /// Note that the Pedersen hash does not provide pseudo-randomness, which
    /// is needed for the BLS signature scheme to be secure. So, we assume that
    /// the input hash is already pseudo-random.
    pub fn hash_to_g1(hash: SigHash) -> G1Projective {
        // Transform the hash into bits.
        let bits = bytes_to_bits(hash.as_bytes());

        // Get the generators for the Pedersen hash.
        let generators = pedersen_generators(2);

        // Calculate the Pedersen hash.
        pedersen_hash(bits, generators)
    }

    /// Transforms a signature into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine
    /// form), one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedSignature {
        CompressedSignature::from(self.signature)
    }

    /// Multiplies a Signature by a u16. It's useful when you need to
    /// validator's signature by its number of slots.
    pub fn multiply(&self, x: u16) -> Self {
        let signature = self.signature.mul(&[x as u64]);
        Signature {
            signature,
            compressed: CompressedSignature::from(signature),
        }
    }
}

impl Eq for Signature {}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.signature.eq(&other.signature)
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.compress().to_hex())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Signature({})", &::hex::encode(self.compress().as_ref()))
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    // TODO: Replace this with a generic serialization using `ToHex` and `FromHex`.

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::{CompressedSignature, Signature};

    impl Serialize for Signature {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Serialize::serialize(&self.compress(), serializer)
        }
    }

    impl<'de> Deserialize<'de> for Signature {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let compressed: CompressedSignature = Deserialize::deserialize(deserializer)?;
            compressed.uncompress().map_err(Error::custom)
        }
    }
}
