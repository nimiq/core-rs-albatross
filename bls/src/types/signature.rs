use std::fmt;

use ark_ec::{AffineRepr, Group};
use ark_ff::{One, PrimeField, ToConstraintField};
use ark_mnt6_753::{Fq, G1Affine, G1Projective};
use nimiq_hash::{blake2s::Blake2sWithParameterBlock, HashOutput};

use crate::{CompressedSignature, SigHash};

#[derive(Clone, Copy, Default)]
#[cfg_attr(feature = "serde-derive", derive(nimiq_hash_derive::SerializeContent))]
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
    /// Maximum size in bytes for a single `Signature` in binary serialization.
    pub const SIZE: usize = CompressedSignature::SIZE;

    /// Maps an hash to a elliptic curve point in the G1 group, it is known as
    /// "hash-to-curve". It is required to create signatures. We use the
    /// try-and-increment method to create the EC point.
    pub fn hash_to_g1(hash: SigHash) -> G1Projective {
        // This extends the seed using the Blake2X algorithm.
        // See https://blake2.net/blake2x.pdf for more details.
        // We need 96 bytes of output for the generator that we are going to create.
        let mut bytes = vec![];

        for i in 0..3 {
            let blake2x = Blake2sWithParameterBlock::new_blake2x(i, 0xffff);

            let mut result = blake2x.evaluate(hash.as_bytes());

            bytes.append(&mut result);
        }

        // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve
        // point. At this time, it is not guaranteed to be a valid point. A quirk of this code is that
        // we need to set the most significant 16 bits to zero. The reason for this is that the field for
        // the MNT6-753 curve is not exactly 753 bits, it is a bit smaller. This means that if we try
        // to create a field element from 753 random bits, we may get an invalid value back (in this
        // case it is just all zeros). There are two options to deal with this:
        // 1) To create finite field elements, using 753 random bits, in a loop until a valid one
        //    is created.
        // 2) Use only 752 random bits to create a finite field element. This will guaranteedly
        //    produce a valid element on the first try, but will reduce the entropy of the EC
        //    point generation by one bit.
        // We chose the second one because we believe the entropy reduction is not significant enough.
        // Since we have 768 bits per generator but only need 752 bits, we set the most significant 16 bits (768-752=16)
        // to zero.

        // The y-coordinate is at the most significant bit (interpreted as little endian). We convert it to a boolean.
        let bytes_len = bytes.len();
        let y_bit = (bytes[bytes_len - 1] >> 7) & 1 == 1;

        // Because of the previous explanation, we need to remove the whole last two bytes.
        let max_size = ((Fq::MODULUS_BIT_SIZE - 1) / 8) as usize;
        bytes.truncate(max_size);

        let x_coordinates = ToConstraintField::to_field_elements(&bytes).unwrap();
        assert_eq!(x_coordinates.len(), 1);
        let mut x_coordinate = x_coordinates[0];

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        loop {
            let point = G1Affine::get_point_from_x_unchecked(x_coordinate, y_bit);

            if let Some(point) = point {
                // We don't need to scale by the cofactor since MNT6-753 has a cofactor of one.
                let g1 = point.into_group();
                return g1;
            }

            x_coordinate += &Fq::one();
        }
    }

    /// Transforms a signature into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine
    /// form), one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedSignature {
        self.compressed
    }

    /// Multiplies a Signature by a u16. It's useful when you need to
    /// multiply a validator's signature by its number of slots.
    #[must_use]
    pub fn multiply(&self, x: u16) -> Self {
        let signature = self.signature.mul_bigint([x as u64]);
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

impl From<G1Projective> for Signature {
    fn from(signature: G1Projective) -> Self {
        let compressed = CompressedSignature::from(signature);

        Signature {
            signature,
            compressed,
        }
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
